use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

use super::runtime::{random_id, write_new};

pub struct Profile {
    pub directory: PathBuf,
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

pub fn prepare(root: &Path, workspace_id: &str) -> Result<Profile, String> {
    let directory = root.join("workspaces").join(key(workspace_id));
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
        "cert":false, "cookie-suffix":format!("workspace-{}", key(workspace_id)), "disable-telemetry":true,
        "disable-update-check":true, "disable-proxy":true
    });
    let config_path = directory.join("config.yaml");
    write_new(&config_path, b"{}", true)?;
    fs::write(config_path, config.to_string()).map_err(|error| error.to_string())?;
    Ok(Profile {
        directory,
        password,
    })
}

pub fn task_workspace(root: &Path, task_id: &str, folders: &[PathBuf]) -> Result<PathBuf, String> {
    let directory = root.join("sessions").join(key(task_id));
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
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
    Ok(workspace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_profiles_share_preferences_and_keep_separate_global_state() {
        let root = std::env::temp_dir().join(random_id().unwrap());
        let first = prepare(&root, "../one").unwrap();
        let second = prepare(&root, "two").unwrap();
        assert_ne!(first.directory, second.directory);
        assert!(first.directory.starts_with(root.join("workspaces")));
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
        let resumed = prepare(&root, "../one").unwrap();
        assert_eq!(
            fs::read(resumed.directory.join("data/User/state.db")).unwrap(),
            b"one"
        );
        assert!(!second.directory.join("data/User/state.db").exists());
        let config: serde_json::Value =
            serde_json::from_slice(&fs::read(resumed.directory.join("config.yaml")).unwrap())
                .unwrap();
        assert_eq!(config["bind-addr"], "127.0.0.1:53172");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tasks_keep_stable_workspaces_and_preserve_user_settings() {
        let root = std::env::temp_dir().join(random_id().unwrap());
        let first = task_workspace(&root, "a", &["/a".into(), "/b".into()]).unwrap();
        let second = task_workspace(&root, "b", &["/c".into()]).unwrap();
        assert_ne!(first, second);
        let mut document: serde_json::Value =
            serde_json::from_slice(&fs::read(&first).unwrap()).unwrap();
        document["settings"] = json!({"editor.fontSize": 18});
        fs::write(&first, document.to_string()).unwrap();
        assert_eq!(
            task_workspace(&root, "a", &["/a".into(), "/b".into()]).unwrap(),
            first
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(&first).unwrap()).unwrap(),
            document
        );
        assert!(task_workspace(&root, "a", &["/other".into()]).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
