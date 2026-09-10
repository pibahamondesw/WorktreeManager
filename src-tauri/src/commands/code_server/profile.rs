use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

use super::runtime::{random_id, write_new};

pub struct Profile {
    pub directory: PathBuf,
    pub workspace: PathBuf,
    pub password: String,
}

pub fn key(task_id: &str) -> String {
    format!("{:x}", md5::compute(task_id))
}

pub fn canonical_folders(folders: Vec<String>) -> Result<Vec<PathBuf>, String> {
    if folders.is_empty() {
        return Err("No folders to open".into());
    }
    folders
        .into_iter()
        .map(|folder| {
            let path = fs::canonicalize(&folder)
                .map_err(|error| format!("Cannot open {folder}: {error}"))?;
            if !path.is_dir() {
                return Err(format!("Not a folder: {folder}"));
            }
            Ok(path)
        })
        .collect()
}

pub fn prepare(root: &Path, task_id: &str, folders: &[PathBuf]) -> Result<Profile, String> {
    let directory = root.join("sessions").join(key(task_id));
    let user = directory.join("data/User");
    let preferences = root.join("preferences");
    fs::create_dir_all(&user)
        .and_then(|_| fs::create_dir_all(&preferences))
        .map_err(|error| error.to_string())?;
    let defaults = json!({
        "telemetry.telemetryLevel": "off",
        "chat.commandCenter.enabled": false,
        "workbench.secondarySideBar.defaultVisibility": "hidden",
        "workbench.startupEditor": "none"
    });
    for (name, content) in [
        ("settings.json", defaults.to_string()),
        ("keybindings.json", "[]".into()),
    ] {
        let source = preferences.join(name);
        write_new(&source, content.as_bytes(), false)?;
        let target = user.join(name);
        if !target.try_exists().map_err(|error| error.to_string())? {
            std::os::unix::fs::symlink(&source, &target).map_err(|error| error.to_string())?;
        }
        if fs::canonicalize(&target).ok() != fs::canonicalize(&source).ok() {
            return Err(format!(
                "Shared preferences were replaced at {}. Restore the link before reopening.",
                target.display()
            ));
        }
    }
    let workspace = directory.join("task.code-workspace");
    let document =
        json!({"folders": folders.iter().map(|path| json!({"path":path})).collect::<Vec<_>>()});
    if workspace.exists() {
        let previous: serde_json::Value =
            serde_json::from_slice(&fs::read(&workspace).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        if previous["folders"] != document["folders"] {
            return Err("This editor profile belongs to different folders. Its workspace cannot be switched while restoring the task.".into());
        }
    } else {
        fs::write(&workspace, document.to_string()).map_err(|error| error.to_string())?;
    }
    let password = random_id()?;
    let port_path = directory.join("port");
    let port = match fs::read_to_string(&port_path) {
        Ok(port) => port
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .ok_or("The saved editor port is invalid")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error.to_string()),
    };
    let config = json!({
        "bind-addr":format!("127.0.0.1:{port}"), "auth":"password", "password":password,
        "cert":false, "cookie-suffix":key(task_id), "disable-telemetry":true,
        "disable-update-check":true, "disable-proxy":true
    });
    let config_path = directory.join("config.yaml");
    write_new(&config_path, b"{}", true)?;
    fs::write(config_path, config.to_string()).map_err(|error| error.to_string())?;
    Ok(Profile {
        directory,
        workspace,
        password,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_profiles_share_preferences_but_keep_workspace_state_separate() {
        let root = std::env::temp_dir().join(random_id().unwrap());
        fs::create_dir_all(&root).unwrap();
        let first = prepare(&root, "../one", std::slice::from_ref(&root)).unwrap();
        let second = prepare(&root, "two", std::slice::from_ref(&root)).unwrap();
        assert_ne!(first.directory, second.directory);
        assert!(first.directory.starts_with(root.join("sessions")));
        assert_ne!(first.password, second.password);
        fs::write(
            first.directory.join("data/User/settings.json"),
            "{\"editor.fontSize\":18}",
        )
        .unwrap();
        assert_eq!(
            fs::read(second.directory.join("data/User/settings.json")).unwrap(),
            b"{\"editor.fontSize\":18}"
        );
        fs::write(first.directory.join("data/User/state.db"), "one").unwrap();
        fs::write(first.directory.join("port"), "53172").unwrap();
        assert!(!second.directory.join("data/User/state.db").exists());
        let resumed = prepare(&root, "../one", std::slice::from_ref(&root)).unwrap();
        assert_eq!(
            fs::read(resumed.directory.join("data/User/state.db")).unwrap(),
            b"one"
        );
        let config: serde_json::Value =
            serde_json::from_slice(&fs::read(resumed.directory.join("config.yaml")).unwrap())
                .unwrap();
        assert_eq!(config["bind-addr"], "127.0.0.1:53172");
        assert!(prepare(&root, "../one", &[root.join("other")]).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
