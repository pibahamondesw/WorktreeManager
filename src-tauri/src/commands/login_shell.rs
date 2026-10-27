//! The one way WorktreeManager starts a process through the user's login shell: agent chats and
//! terminals, CLI probes, dependency installs and repository scripts. GUI launches inherit a
//! minimal environment, so every launch sources the user's profiles first. Callers only say what
//! runs, where and with which environment; dynamic values reach the shell as positional
//! arguments, never as script text, so nothing needs quoting.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, PoisonError};

use super::git::GIT_ENV_SCRUB;

const SHELL: &str = "/bin/zsh";
const SCRIPT_NAME: &str = "zsh";

/// Bash-oriented profiles sourced under zsh routinely write noise (e.g. a stale `. <cargo/env>`
/// line), so their stderr is always discarded.
const PROFILES: &str = r#"export PATH="$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
{ [ -f "$HOME/.zprofile" ] && source "$HOME/.zprofile"; [ -f "$HOME/.zshrc" ] && source "$HOME/.zshrc"; [ -f "$HOME/.profile" ] && source "$HOME/.profile"; [ -f "$HOME/.bash_profile" ] && source "$HOME/.bash_profile"; }"#;

/// The profiles the launcher sources run in the script's own scope, where they may reset its
/// positional arguments or change directory.
const SAVE_LAUNCH_STATE: &str = r#"wtm_launch_arguments=("$@") wtm_launch_dir=$PWD"#;
const RESTORE_LAUNCH_STATE: &str = r#"[ "$PWD" = "$wtm_launch_dir" ] || cd -- "$wtm_launch_dir"
set -- "${wtm_launch_arguments[@]}"; unset wtm_launch_arguments wtm_launch_dir"#;

pub(crate) const SHIM_DIR_ENV: &str = "WTM_SHIM_DIR";
const PREFER_SHIMS: &str = r#"[ -z "$WTM_SHIM_DIR" ] || ! command -v doppler >/dev/null 2>&1 || PATH="$WTM_SHIM_DIR:$PATH""#;

/// A launch for the embedded PTY.
pub struct LaunchSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub env_remove: Vec<String>,
    pub cwd: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LoginShell {
    script: String,
    arguments: Vec<String>,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
}

#[derive(Clone, Copy)]
enum ProfileOutput {
    Hidden,
    Shown,
}

impl LoginShell {
    /// Runs `program` with `args` exactly as given.
    pub fn program<I, S>(program: &str, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::script(r#"exec "$@""#).args([program]).args(args)
    }

    /// Runs a script written by the app. Dynamic values go through `args` and are read as `"$1"`,
    /// `"$2"`, … or `"$@"`.
    pub fn script(script: &str) -> Self {
        Self {
            script: script.to_string(),
            arguments: Vec::new(),
            cwd: None,
            env: Vec::new(),
        }
    }

    /// Runs a repository setup or teardown script verbatim, stopping at its first failing line.
    /// The script is authored by the user or a repository they approved; task context reaches it
    /// only through environment variables. When `doppler` resolves after the profiles,
    /// `WTM_SHIM_DIR` goes first on the `PATH` they set.
    pub fn repo_script(script: &str) -> Self {
        Self::script(&format!("{PREFER_SHIMS}\nset -e\n{script}"))
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.arguments
            .extend(args.into_iter().map(|arg| arg.as_ref().to_string()));
        self
    }

    pub fn current_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.cwd = Some(dir.as_ref().to_path_buf());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn envs(mut self, vars: impl IntoIterator<Item = (String, String)>) -> Self {
        self.env.extend(vars);
        self
    }

    /// A process whose output the app reads. Output of the profiles the launcher sources is
    /// discarded so it never mixes with the program's (zsh's own login files still print), and
    /// Git variables inherited by the app never redirect it to another repository.
    pub fn command(&self) -> Command {
        let mut command = Command::new(SHELL);
        command
            .arg("-lc")
            .arg(self.shell_script(ProfileOutput::Hidden))
            .arg(SCRIPT_NAME)
            .args(&self.arguments)
            .envs(self.env.iter().map(|(key, value)| (key, value)));
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd).env("PWD", cwd);
        }
        for variable in GIT_ENV_SCRUB {
            command.env_remove(variable);
        }
        command
    }

    pub fn output(&self) -> io::Result<Output> {
        self.command().output()
    }

    /// A launch for an interactive terminal, where profile output stays visible as in any shell.
    pub fn terminal(self) -> LaunchSpec {
        let mut args = vec![
            "-lc".to_string(),
            self.shell_script(ProfileOutput::Shown),
            SCRIPT_NAME.to_string(),
        ];
        args.extend(self.arguments);
        LaunchSpec {
            program: SHELL.to_string(),
            args,
            env: self.env,
            env_remove: GIT_ENV_SCRUB.iter().map(|name| name.to_string()).collect(),
            cwd: self
                .cwd
                .map_or_else(|| "/".to_string(), |dir| dir.to_string_lossy().into_owned()),
        }
    }

    fn shell_script(&self, profile_output: ProfileOutput) -> String {
        let redirect = match profile_output {
            ProfileOutput::Hidden => ">/dev/null 2>&1",
            ProfileOutput::Shown => "2>/dev/null",
        };
        format!(
            "{SAVE_LAUNCH_STATE}\n{PROFILES} {redirect}\n{RESTORE_LAUNCH_STATE}\n{}",
            self.script
        )
    }
}

static AVAILABLE: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// True when `program` resolves after the profiles. A found program is remembered for the rest of
/// the process so launches don't source the profiles once more just to check; a missing one is
/// probed again on every call. The Doctor refresh replaces what is remembered.
pub fn cli_available(program: &str) -> bool {
    if AVAILABLE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(program)
    {
        return true;
    }
    let found = availability_probe(program)
        .output()
        .is_ok_and(|output| output.status.success());
    remember_availability(program, found);
    found
}

pub(crate) fn remember_availability(program: &str, found: bool) {
    let mut available = AVAILABLE.lock().unwrap_or_else(PoisonError::into_inner);
    if found {
        available.insert(program.to_string());
    } else {
        available.remove(program);
    }
}

fn availability_probe(program: &str) -> LoginShell {
    LoginShell::script(r#"command -v "$1""#).args([program])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    struct Home(PathBuf);

    impl Home {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("wtm-login-shell-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("bin")).unwrap();
            fs::create_dir_all(root.join("work dir")).unwrap();
            fs::write(
                root.join(".zshrc"),
                "echo profile-noise\necho profile-error >&2\nset -- clobbered\ncd /\nexport PATH=\"$HOME/bin:$PATH\"\n",
            )
            .unwrap();
            executable(
                &root.join("bin/wtm-print-args"),
                "#!/bin/sh\nfor arg in \"$@\"; do printf '[%s]\\n' \"$arg\"; done\npwd -P\n",
            );
            Self(fs::canonicalize(root).unwrap())
        }

        fn work_dir(&self) -> PathBuf {
            self.0.join("work dir")
        }

        fn run(&self, launch: &LoginShell) -> Output {
            launch
                .command()
                .env("HOME", &self.0)
                .env_remove("ZDOTDIR")
                .output()
                .unwrap()
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn executable(path: &Path, contents: &str) {
        fs::write(path, contents).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn stdout(output: &Output) -> String {
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    #[test]
    fn programs_resolve_through_profiles_and_receive_arguments_verbatim() {
        let home = Home::new("program");
        let hostile = ["it's", "$(touch pwned)", "a b", "; exit 3", "`id`"];
        let output =
            home.run(&LoginShell::program("wtm-print-args", hostile).current_dir(home.work_dir()));
        assert!(output.status.success());
        assert_eq!(
            stdout(&output),
            format!(
                "[it's]\n[$(touch pwned)]\n[a b]\n[; exit 3]\n[`id`]\n{}\n",
                home.work_dir().display()
            )
        );
        assert!(!home.work_dir().join("pwned").exists());
        assert!(String::from_utf8_lossy(&output.stderr).is_empty());
    }

    #[test]
    fn scripts_read_dynamic_values_as_positional_arguments() {
        let home = Home::new("script");
        let output = home.run(
            &LoginShell::script(r#"printf '%s|%s|%s' "$#" "$1" "$FROM_ENV""#)
                .args(["a'b $(c)"])
                .env("FROM_ENV", "set"),
        );
        assert_eq!(stdout(&output), "1|a'b $(c)|set");
    }

    #[test]
    fn captured_launches_keep_the_working_directory_as_given() {
        let home = Home::new("logical-cwd");
        let link = home.0.join("linked work dir");
        std::os::unix::fs::symlink(home.work_dir(), &link).unwrap();
        let output = home.run(&LoginShell::script(r#"printf %s "$PWD""#).current_dir(&link));
        assert_eq!(stdout(&output), link.to_string_lossy());
    }

    #[test]
    fn captured_launches_scrub_inherited_git_environment() {
        let command = LoginShell::program("git", ["status"]).command();
        let removed: Vec<_> = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect();
        for variable in GIT_ENV_SCRUB {
            assert!(removed.contains(&variable.to_string()), "{variable}");
        }
    }

    #[test]
    fn repo_scripts_stop_at_the_first_failing_line() {
        let home = Home::new("repo-script");
        let output = home.run(&LoginShell::repo_script("echo one\nfalse\necho never"));
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(stdout(&output), "one\n");
    }

    #[test]
    fn repo_scripts_prefer_the_shim_dir_over_profile_paths() {
        let home = Home::new("repo-script-shims");
        let shims = home.0.join("shims");
        fs::create_dir_all(&shims).unwrap();
        executable(&shims.join("doppler"), "#!/bin/sh\necho shim\n");
        fs::write(
            home.0.join(".zshrc"),
            "export PATH=\"$HOME/bin:/usr/bin:/bin\"\n",
        )
        .unwrap();
        let script = LoginShell::repo_script("doppler 2>/dev/null || echo absent")
            .env(SHIM_DIR_ENV, shims.to_string_lossy());
        assert_eq!(stdout(&home.run(&script)), "absent\n");

        executable(&home.0.join("bin/doppler"), "#!/bin/sh\necho real\n");
        let script = LoginShell::repo_script("doppler");
        assert_eq!(stdout(&home.run(&script)), "real\n");
        let output = home.run(&script.env(SHIM_DIR_ENV, shims.to_string_lossy()));
        assert_eq!(stdout(&output), "shim\n");
    }

    #[test]
    fn terminal_launches_show_profile_output_and_carry_cwd_env_and_scrub() {
        let home = Home::new("terminal");
        let spec = LoginShell::program("wtm-print-args", ["x y"])
            .current_dir(home.work_dir())
            .env("SESSION", "s1")
            .terminal();
        assert_eq!(spec.program, SHELL);
        assert_eq!(spec.cwd, home.work_dir().to_string_lossy());
        assert_eq!(spec.env, vec![("SESSION".to_string(), "s1".to_string())]);
        assert_eq!(spec.env_remove.len(), GIT_ENV_SCRUB.len());
        let output = Command::new(&spec.program)
            .args(&spec.args)
            .current_dir(&spec.cwd)
            .env("HOME", &home.0)
            .env_remove("ZDOTDIR")
            .output()
            .unwrap();
        assert_eq!(
            stdout(&output),
            format!("profile-noise\n[x y]\n{}\n", home.work_dir().display())
        );
        assert_eq!(LoginShell::script("true").terminal().cwd, "/");
    }

    #[test]
    fn availability_resolves_through_profiles() {
        let home = Home::new("availability");
        assert!(home
            .run(&availability_probe("wtm-print-args"))
            .status
            .success());
        assert!(!home
            .run(&availability_probe("wtm-not-installed"))
            .status
            .success());
    }

    #[test]
    fn found_programs_are_remembered_until_a_refresh_says_otherwise() {
        let home = Home::new("remembered");
        let program = home.0.join("bin/wtm-print-args");
        let program = program.to_str().unwrap();
        assert!(cli_available(program));
        fs::remove_file(program).unwrap();
        assert!(cli_available(program));
        remember_availability(program, false);
        assert!(!cli_available(program));
        assert!(!cli_available(program));
    }
}
