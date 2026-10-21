//! Atomic replacement of files the user may own, edit or symlink.
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub(crate) struct UserFile<'a> {
    path: &'a Path,
    unchanged_from: Option<&'a [u8]>,
    private: bool,
}

impl<'a> UserFile<'a> {
    pub(crate) fn new(path: &'a Path) -> Self {
        Self {
            path,
            unchanged_from: None,
            private: false,
        }
    }

    /// Refuse to replace the file when it no longer matches what the caller read.
    /// A missing file reads as empty.
    pub(crate) fn unchanged_from(mut self, original: &'a (impl AsRef<[u8]> + ?Sized)) -> Self {
        self.unchanged_from = Some(original.as_ref());
        self
    }

    /// Create a missing file readable only by the user.
    pub(crate) fn private(mut self) -> Self {
        self.private = true;
        self
    }

    /// Follows symlinks and keeps the existing permissions; a reader sees either the old or
    /// the new contents, never a partial write.
    pub(crate) fn replace(self, contents: impl AsRef<[u8]>) -> Result<(), String> {
        let target = resolve_target(self.path)?;
        let temporary = temporary_sibling(&target)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(if self.private { 0o600 } else { 0o666 })
            .open(&temporary)
            .map_err(|e| format!("Cannot write {}: {e}", self.path.display()))?;
        let result = self.finish(&mut file, &temporary, &target, contents.as_ref());
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn finish(
        &self,
        file: &mut File,
        temporary: &Path,
        target: &Path,
        contents: &[u8],
    ) -> Result<(), String> {
        let display = self.path.display();
        if let Ok(metadata) = fs::metadata(target) {
            file.set_permissions(metadata.permissions())
                .map_err(|e| format!("Cannot preserve permissions of {display}: {e}"))?;
        }
        file.write_all(contents)
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("Cannot write {display}: {e}"))?;
        if let Some(original) = self.unchanged_from {
            if read_or_empty(target)? != original {
                return Err(format!(
                    "{display} changed while it was being updated; retry"
                ));
            }
        }
        fs::rename(temporary, target).map_err(|e| format!("Cannot replace {display}: {e}"))
    }
}

/// A user's symlink stays in place and its target is replaced; a dangling link is an error
/// rather than a missing file to create.
fn resolve_target(path: &Path) -> Result<PathBuf, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(path)
            .map_err(|e| format!("Cannot resolve symlink {}: {e}", path.display())),
        _ => Ok(path.to_path_buf()),
    }
}

fn temporary_sibling(target: &Path) -> Result<PathBuf, String> {
    let name = target
        .file_name()
        .ok_or_else(|| format!("Not a file path: {}", target.display()))?;
    Ok(target.with_file_name(format!(
        ".{}.{}.wm.tmp",
        name.to_string_lossy(),
        uuid::Uuid::new_v4().simple()
    )))
}

fn read_or_empty(path: &Path) -> Result<Vec<u8>, String> {
    match fs::read(path) {
        Ok(contents) => Ok(contents),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("Cannot read {}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("wm-user-file-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn entries(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(&self.0)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn creates_and_replaces_without_leaving_temporary_files() {
        let directory = TestDirectory::new();
        let path = directory.0.join("config.json");

        UserFile::new(&path).replace("first").unwrap();
        UserFile::new(&path).replace("second").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        assert_eq!(directory.entries(), vec!["config.json"]);
    }

    #[test]
    fn temporary_names_are_unique_per_write() {
        let target = Path::new("/somewhere/config.json");
        let first = temporary_sibling(target).unwrap();
        let second = temporary_sibling(target).unwrap();

        assert_ne!(first, second);
        assert_eq!(first.parent(), target.parent());
        assert!(first
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".config.json."));
    }

    #[test]
    fn concurrent_writers_never_share_a_temporary_file() {
        let directory = TestDirectory::new();
        let path = directory.0.join("config.json");
        let writers: Vec<_> = (0..8)
            .map(|index| {
                let path = path.clone();
                std::thread::spawn(move || UserFile::new(&path).replace(format!("writer {index}")))
            })
            .collect();

        for writer in writers {
            writer.join().unwrap().unwrap();
        }

        assert!(fs::read_to_string(&path).unwrap().starts_with("writer "));
        assert_eq!(directory.entries(), vec!["config.json"]);
    }

    #[test]
    fn failed_rename_keeps_the_destination_and_removes_the_temporary_file() {
        let directory = TestDirectory::new();
        let path = directory.0.join("config.json");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("preserve"), "original").unwrap();

        let error = UserFile::new(&path).replace("replacement").unwrap_err();

        assert!(error.starts_with("Cannot replace"), "{error}");
        assert_eq!(
            fs::read_to_string(path.join("preserve")).unwrap(),
            "original"
        );
        assert_eq!(directory.entries(), vec!["config.json"]);
    }

    #[test]
    fn failed_temporary_write_keeps_the_original() {
        let directory = TestDirectory::new();
        let path = directory.0.join("config.json");
        fs::write(&path, "original").unwrap();
        fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o500)).unwrap();

        let result = UserFile::new(&path).replace("replacement");
        fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o700)).unwrap();

        assert!(result.unwrap_err().starts_with("Cannot write"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "original");
        assert_eq!(directory.entries(), vec!["config.json"]);
    }

    #[test]
    fn replaces_the_target_of_a_symlink_and_keeps_the_link() {
        let directory = TestDirectory::new();
        let target = directory.0.join("dotfiles-config.json");
        let link = directory.0.join("config.json");
        fs::write(&target, "original").unwrap();
        symlink(&target, &link).unwrap();

        UserFile::new(&link).replace("replacement").unwrap();

        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_to_string(&target).unwrap(), "replacement");
        assert_eq!(
            directory.entries(),
            vec!["config.json", "dotfiles-config.json"]
        );
    }

    #[test]
    fn refuses_to_replace_a_dangling_symlink() {
        let directory = TestDirectory::new();
        let link = directory.0.join("config.json");
        symlink(directory.0.join("missing.json"), &link).unwrap();

        assert!(UserFile::new(&link).replace("replacement").is_err());

        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(directory.entries(), vec!["config.json"]);
    }

    #[test]
    fn preserves_existing_permissions_and_creates_private_files() {
        let directory = TestDirectory::new();
        let existing = directory.0.join("existing");
        fs::write(&existing, "original").unwrap();
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o640)).unwrap();
        let private = directory.0.join("private");

        UserFile::new(&existing)
            .private()
            .replace("replacement")
            .unwrap();
        UserFile::new(&private).private().replace("secret").unwrap();

        assert_eq!(mode(&existing), 0o640);
        assert_eq!(mode(&private), 0o600);
    }

    #[test]
    fn refuses_to_overwrite_changes_made_since_the_read() {
        let directory = TestDirectory::new();
        let path = directory.0.join("config.json");
        fs::write(&path, "edited elsewhere").unwrap();

        let error = UserFile::new(&path)
            .unchanged_from("read earlier")
            .replace("replacement")
            .unwrap_err();

        assert!(
            error.contains("changed while it was being updated"),
            "{error}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "edited elsewhere");
        assert_eq!(directory.entries(), vec!["config.json"]);
    }

    #[test]
    fn unchanged_check_reports_an_unreadable_target() {
        let directory = TestDirectory::new();
        let path = directory.0.join("config.json");
        fs::create_dir(&path).unwrap();

        let error = UserFile::new(&path)
            .unchanged_from("")
            .replace("replacement")
            .unwrap_err();

        assert!(error.starts_with("Cannot read"), "{error}");
        assert_eq!(directory.entries(), vec!["config.json"]);
    }

    #[test]
    fn unchanged_check_accepts_the_read_contents_and_treats_missing_as_empty() {
        let directory = TestDirectory::new();
        let path = directory.0.join("config.json");

        UserFile::new(&path)
            .unchanged_from("")
            .replace("created")
            .unwrap();
        UserFile::new(&path)
            .unchanged_from("created")
            .replace("updated")
            .unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "updated");
    }
}
