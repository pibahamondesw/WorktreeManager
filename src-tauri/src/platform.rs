//! The operating-system differences the rest of the app relies on, kept in one place so callers
//! stay portable: process-tree signalling, file locks, local sockets, Unix permission bits,
//! symlinks and raw path bytes.
//!
//! On Unix every helper is a thin wrapper over the call the code used before. On Windows,
//! process groups map to process trees (`taskkill /T`), file locks to `LockFileEx`, local sockets
//! to Windows' own `AF_UNIX` support, permission bits are synthesized from the read-only flag,
//! and setting them is a no-op.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Local stream sockets: the std types on Unix, Windows' `AF_UNIX` sockets (Windows 10 1803+)
/// through `uds_windows`, with the same API.
#[cfg(unix)]
pub use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
pub use uds_windows::{UnixListener, UnixStream};

/// The private per-user directory the CLI bridge socket and instance lock live in:
/// `/tmp/wtm-<uid>` on Unix, `%LOCALAPPDATA%\WorktreeManager\run` on Windows.
pub fn socket_dir() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from(format!("/tmp/wtm-{}", unsafe { libc::geteuid() }))
    }
    #[cfg(windows)]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        base.join("WorktreeManager").join("run")
    }
}

/// Creates `path` if needed and checks that only the current user can use it. On Windows the
/// directory sits in the user's profile, whose ACL already keeps other users out.
pub fn prepare_private_dir(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err("Cannot create socket directory".into()),
        }
        let metadata = fs::symlink_metadata(path).map_err(|_| "Cannot inspect socket directory")?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err("Socket directory must be private and owned by the current user".into());
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        fs::create_dir_all(path).map_err(|_| "Cannot create socket directory")?;
        let metadata = fs::symlink_metadata(path).map_err(|_| "Cannot inspect socket directory")?;
        if !metadata.is_dir() {
            return Err("Socket directory must be a directory".into());
        }
        Ok(())
    }
}

/// Opens (creating) a lock file readable only by its owner, refusing to follow a symlink on Unix.
pub fn open_lock_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

/// Takes an exclusive lock on `file`, waiting for it (`flock` / `LockFileEx`). Released when the
/// file is closed.
pub fn lock_exclusive(file: &fs::File) -> io::Result<()> {
    lock(file, true)
        .then_some(())
        .ok_or_else(io::Error::last_os_error)
}

/// Takes an exclusive lock on `file` if no one else holds it.
pub fn try_lock_exclusive(file: &fs::File) -> bool {
    lock(file, false)
}

fn lock(file: &fs::File, wait: bool) -> bool {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let flags = if wait {
            libc::LOCK_EX
        } else {
            libc::LOCK_EX | libc::LOCK_NB
        };
        unsafe { libc::flock(file.as_raw_fd(), flags) == 0 }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
        };
        use windows_sys::Win32::System::IO::OVERLAPPED;
        let flags = if wait {
            LOCKFILE_EXCLUSIVE_LOCK
        } else {
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY
        };
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        unsafe {
            LockFileEx(
                file.as_raw_handle(),
                flags,
                0,
                u32::MAX,
                u32::MAX,
                &mut overlapped,
            ) != 0
        }
    }
}

/// Whether the peer of `stream` hung up, without consuming or waiting for any data.
pub fn peer_closed(stream: &UnixStream) -> bool {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let mut byte = 0u8;
        let read = unsafe {
            libc::recv(
                stream.as_raw_fd(),
                (&raw mut byte).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        read == 0 || (read < 0 && io::Error::last_os_error().kind() != io::ErrorKind::WouldBlock)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Networking::WinSock::{recv, WSAGetLastError, MSG_PEEK};
        const WSAEWOULDBLOCK: i32 = 10035;
        if stream.set_nonblocking(true).is_err() {
            return false;
        }
        let mut byte = 0u8;
        let read = unsafe { recv(stream.as_raw_socket() as usize, &raw mut byte, 1, MSG_PEEK) };
        let would_block = read < 0 && unsafe { WSAGetLastError() } == WSAEWOULDBLOCK;
        let _ = stream.set_nonblocking(false);
        read == 0 || (read < 0 && !would_block)
    }
}

/// The signal that ended a process (`128 + signal` is the shell's exit code for it); 0 when it
/// exited normally or on Windows, which has no signals.
pub fn terminating_signal(status: &std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.signal().unwrap_or_default()
    }
    #[cfg(windows)]
    {
        let _ = status;
        0
    }
}

#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Starts `command` as the leader of its own process group (Unix) or process tree with no
/// console window (Windows), so [`signal_tree`] reaches everything it spawns.
pub fn own_group(command: &mut Command) -> &mut Command {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
    }
}

/// Keeps a console program launched from the GUI from flashing a console window on Windows.
/// No-op elsewhere.
pub fn hide_console(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Asks the process group led by `pid` to stop (`SIGTERM`), or stops it outright when `force`
/// (`SIGKILL`). On Windows both end the whole process tree. A group that is already gone is not
/// an error.
pub fn signal_tree(pid: u32, force: bool) -> io::Result<()> {
    #[cfg(unix)]
    {
        signal_unix(-(pid as i32), force)
    }
    #[cfg(windows)]
    {
        let _ = force;
        taskkill(pid, true)
    }
}

/// Like [`signal_tree`] for the single process `pid`.
pub fn signal_process(pid: u32, force: bool) -> io::Result<()> {
    #[cfg(unix)]
    {
        signal_unix(pid as i32, force)
    }
    #[cfg(windows)]
    {
        let _ = force;
        taskkill(pid, false)
    }
}

/// Whether any process of the group led by `pid` is still running.
pub fn tree_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let signalled = unsafe { libc::kill(-(pid as i32), 0) == 0 };
        signalled || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
    #[cfg(windows)]
    {
        process_alive(pid)
    }
}

/// Whether the process `pid` is still running.
pub fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        const STILL_ACTIVE: u32 = 259;
        const ERROR_INVALID_PARAMETER: u32 = 87;
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                // Access denied still means the process exists.
                return GetLastError() != ERROR_INVALID_PARAMETER;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(handle, &mut code) != 0;
            CloseHandle(handle);
            ok && code == STILL_ACTIVE
        }
    }
}

#[cfg(unix)]
fn signal_unix(target: i32, force: bool) -> io::Result<()> {
    let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
    if unsafe { libc::kill(target, signal) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(windows)]
fn taskkill(pid: u32, tree: bool) -> io::Result<()> {
    if !process_alive(pid) {
        return Ok(());
    }
    let mut command = Command::new("taskkill");
    command.args(["/F", "/PID", &pid.to_string()]);
    if tree {
        command.arg("/T");
    }
    hide_console(&mut command)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    Ok(())
}

/// `PATH` as Windows would give a newly started program: the machine and user entries from the
/// registry (picking up tools installed with winget since the app started), followed by the
/// app's own `PATH`. `None` elsewhere, where the inherited `PATH` is used as is.
pub fn fresh_path() -> Option<std::ffi::OsString> {
    #[cfg(unix)]
    {
        None
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        let machine = registry_string(
            HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
            "Path",
        );
        let user = registry_string(HKEY_CURRENT_USER, "Environment", "Path");
        let current = std::env::var_os("PATH");
        let parts: Vec<std::ffi::OsString> =
            [machine, user, current].into_iter().flatten().collect();
        if parts.is_empty() {
            return None;
        }
        let mut joined = std::ffi::OsString::new();
        for (index, part) in parts.iter().enumerate() {
            if index > 0 {
                joined.push(";");
            }
            joined.push(part);
        }
        Some(joined)
    }
}

/// A `REG_SZ`/`REG_EXPAND_SZ` value, with `%VARIABLES%` expanded.
#[cfg(windows)]
fn registry_string(
    root: windows_sys::Win32::System::Registry::HKEY,
    key: &str,
    value: &str,
) -> Option<std::ffi::OsString> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::System::Registry::{RegGetValueW, RRF_RT_REG_SZ};
    let wide = |text: &str| -> Vec<u16> {
        OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };
    let (key, value) = (wide(key), wide(value));
    // Without RRF_NOEXPAND, RRF_RT_REG_SZ also accepts REG_EXPAND_SZ values and expands them.
    let flags = RRF_RT_REG_SZ;
    let mut size = 0u32;
    let status = unsafe {
        RegGetValueW(
            root,
            key.as_ptr(),
            value.as_ptr(),
            flags,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if status != 0 || size == 0 {
        return None;
    }
    let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
    let status = unsafe {
        RegGetValueW(
            root,
            key.as_ptr(),
            value.as_ptr(),
            flags,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    let length = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    Some(std::ffi::OsString::from_wide(&buffer[..length]))
}

/// The Unix mode of `metadata` (file type and permission bits). On Windows: `0o100644` for
/// files, `0o040755` for directories, without write bits when read-only.
pub fn mode(metadata: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode()
    }
    #[cfg(windows)]
    {
        let mode = if metadata.is_dir() {
            0o040_755
        } else {
            0o100_644
        };
        if metadata.permissions().readonly() {
            mode & !0o222
        } else {
            mode
        }
    }
}

/// Sets the Unix permission bits of `path`. No-op on Windows, which has no equivalent.
pub fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
    }
    #[cfg(windows)]
    {
        let _ = (path, mode);
        Ok(())
    }
}

/// Applies `mode` to a file about to be created (Unix only).
pub fn create_mode(options: &mut fs::OpenOptions, mode: u32) -> &mut fs::OpenOptions {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode)
    }
    #[cfg(windows)]
    {
        let _ = mode;
        options
    }
}

/// Whether `metadata` describes a file the current user can execute. On Windows, any file.
pub fn is_executable(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(windows)]
    {
        metadata.is_file()
    }
}

/// Creates a symbolic link at `link` pointing to `target`. On Windows this needs Developer Mode
/// or administrator rights; without them it fails like any other I/O error.
pub fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link)
    }
}

/// The bytes of `value` as Git and the filesystem see them: raw on Unix, UTF-8 on Windows.
pub fn os_bytes(value: &OsStr) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        value.as_bytes().to_vec()
    }
    #[cfg(windows)]
    {
        value.to_string_lossy().into_owned().into_bytes()
    }
}

/// The inverse of [`os_bytes`].
pub fn os_from_bytes(bytes: &[u8]) -> OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        OsStr::from_bytes(bytes).to_os_string()
    }
    #[cfg(windows)]
    {
        OsString::from(String::from_utf8_lossy(bytes).into_owned())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_private_dir_cannot_be_created_under_a_file() {
        let file = std::env::temp_dir().join(format!("wtm-platform-file-{}", std::process::id()));
        fs::write(&file, "").unwrap();
        let result = prepare_private_dir(&file.join("run"));
        fs::remove_file(&file).unwrap();
        assert_eq!(result.unwrap_err(), "Cannot create socket directory");
    }

    #[test]
    fn signalling_a_process_stops_it() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id();
        assert!(process_alive(pid));
        signal_process(pid, false).unwrap();
        child.wait().unwrap();
        assert!(!process_alive(pid));
        assert!(signal_process(pid, true).is_ok());
    }

    #[test]
    fn signalling_another_users_process_fails() {
        let root = unsafe { libc::geteuid() } == 0;
        assert!(root || signal_process(1, false).is_err());
    }
}
