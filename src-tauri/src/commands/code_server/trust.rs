use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Manager};

use super::{profile, runtime};

const BRIDGE_PATH: &str = "/__worktreemanager_trust";
static STORAGE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Default, Deserialize, Serialize)]
struct StoredTrust {
    revision: u64,
    paths: BTreeSet<String>,
    removed: BTreeSet<String>,
    migrated: BTreeSet<String>,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "camelCase", deny_unknown_fields)]
enum Change {
    Initialize {
        paths: BTreeSet<String>,
    },
    Update {
        added: BTreeSet<String>,
        removed: BTreeSet<String>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    id: u32,
    document_id: String,
    change: Change,
}

#[derive(Serialize)]
struct Snapshot {
    revision: u64,
    paths: BTreeSet<String>,
}

fn validate_paths(paths: &BTreeSet<String>) -> Result<(), String> {
    for value in paths {
        let path = Path::new(value);
        if !path.is_absolute()
            || value.contains('\0')
            || path
                .components()
                .any(|part| matches!(part, Component::ParentDir))
        {
            return Err("Shared workspace trust requires absolute local paths".into());
        }
    }
    Ok(())
}

fn update(root: &Path, task: &str, change: Change) -> Result<Snapshot, String> {
    let _guard = STORAGE_LOCK.lock().map_err(|error| error.to_string())?;
    let directory = root.join("preferences");
    let path = directory.join("workspace-trust.json");
    let mut state: StoredTrust = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| error.to_string())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => StoredTrust::default(),
        Err(error) => return Err(error.to_string()),
    };
    match change {
        Change::Initialize { paths } => {
            validate_paths(&paths)?;
            if state.migrated.insert(profile::key(task)) {
                state
                    .paths
                    .extend(paths.difference(&state.removed).cloned());
            }
        }
        Change::Update { added, removed } => {
            validate_paths(&added)?;
            validate_paths(&removed)?;
            for path in removed {
                state.paths.remove(&path);
                state.removed.insert(path);
            }
            for path in added {
                state.removed.remove(&path);
                state.paths.insert(path);
            }
        }
    }
    state.revision += 1;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let temporary = directory.join(format!("workspace-trust-{}.tmp", runtime::random_id()?));
    let bytes = serde_json::to_vec(&state).map_err(|error| error.to_string())?;
    let result = runtime::write_new(&temporary, &bytes, true)
        .and_then(|_| fs::rename(&temporary, path).map_err(|error| error.to_string()));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok(Snapshot {
        revision: state.revision,
        paths: state.paths,
    })
}

pub struct Bridge {
    origin: tauri::Url,
    token: String,
    task: String,
    label: String,
}

impl Bridge {
    pub fn new(origin: &tauri::Url, task: &str, label: &str) -> Result<Self, String> {
        Ok(Self {
            origin: origin.clone(),
            token: runtime::random_id()?,
            task: task.into(),
            label: label.into(),
        })
    }

    pub fn script(&self, workspace: &Path, folders: &[std::path::PathBuf]) -> String {
        let config = json!({
            "origin": self.origin.origin().ascii_serialization(),
            "endpoint": BRIDGE_PATH,
            "token": self.token,
            "workspace": workspace,
            "folders": folders,
        });
        format!("({})({config});", include_str!("trust.js"))
    }

    pub fn handle(&self, app: &AppHandle, target: &tauri::Url) -> bool {
        if target.origin() != self.origin.origin() || target.path() != BRIDGE_PATH {
            return false;
        }
        let Some(request) = self.request(target) else {
            return true;
        };
        let app = app.clone();
        let task = self.task.clone();
        let label = self.label.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let result = runtime::root(&app).and_then(|root| update(&root, &task, request.change));
            let response = match &result {
                Ok(snapshot) => {
                    json!({"id": request.id, "documentId": request.document_id, "snapshot": snapshot})
                }
                Err(error) => {
                    json!({"id": request.id, "documentId": request.document_id, "error": error})
                }
            };
            if let Some(view) = app.get_webview(&label) {
                let _ = view.eval(format!("window.__worktreeTrustReceive?.({response})"));
            }
            if let Ok(snapshot) = result {
                let notification = json!({"snapshot": snapshot});
                for (name, view) in app.webviews() {
                    if name.starts_with("editor-") && name != label {
                        let _ =
                            view.eval(format!("window.__worktreeTrustReceive?.({notification})"));
                    }
                }
            }
        });
        true
    }

    fn request(&self, target: &tauri::Url) -> Option<Request> {
        if target.origin() != self.origin.origin() || target.path() != BRIDGE_PATH {
            return None;
        }
        let params: std::collections::HashMap<_, _> = target.query_pairs().collect();
        if params.get("token").map(|token| token.as_ref()) != Some(self.token.as_str()) {
            return None;
        }
        let message = params
            .get("message")
            .filter(|message| message.len() <= 1_048_576)?;
        serde_json::from_str(message).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).into()).collect()
    }

    #[test]
    fn shares_additions_and_revocations_without_reimporting_stale_profiles() {
        let root = std::env::temp_dir().join(runtime::random_id().unwrap());
        update(
            &root,
            "one",
            Change::Initialize {
                paths: paths(&["/repos"]),
            },
        )
        .unwrap();
        let second = update(&root, "two", Change::Initialize { paths: paths(&[]) }).unwrap();
        assert_eq!(second.paths, paths(&["/repos"]));
        update(
            &root,
            "one",
            Change::Update {
                added: paths(&["/other"]),
                removed: paths(&[]),
            },
        )
        .unwrap();
        let revoked = update(
            &root,
            "two",
            Change::Update {
                added: paths(&[]),
                removed: paths(&["/repos"]),
            },
        )
        .unwrap();
        assert_eq!(revoked.paths, paths(&["/other"]));
        for task in ["one", "legacy"] {
            let reopened = update(
                &root,
                task,
                Change::Initialize {
                    paths: paths(&["/repos"]),
                },
            )
            .unwrap();
            assert_eq!(reopened.paths, paths(&["/other"]));
        }
        let restored = update(
            &root,
            "two",
            Change::Update {
                added: paths(&["/repos"]),
                removed: paths(&[]),
            },
        )
        .unwrap();
        assert_eq!(restored.paths, paths(&["/other", "/repos"]));
        assert!(restored.revision > revoked.revision);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_invalid_paths_and_preserves_malformed_storage() {
        let root = std::env::temp_dir().join(runtime::random_id().unwrap());
        for value in ["relative", "/repos/../other", "/bad\0path"] {
            assert!(update(
                &root,
                "one",
                Change::Initialize {
                    paths: paths(&[value])
                }
            )
            .is_err());
        }
        let preferences = root.join("preferences");
        fs::create_dir_all(&preferences).unwrap();
        let file = preferences.join("workspace-trust.json");
        fs::write(&file, "broken").unwrap();
        assert!(update(&root, "one", Change::Initialize { paths: paths(&[]) }).is_err());
        assert_eq!(fs::read_to_string(file).unwrap(), "broken");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_tasks_preserve_each_others_decisions() {
        let root = std::env::temp_dir().join(runtime::random_id().unwrap());
        std::thread::scope(|scope| {
            for folder in ["/one", "/two", "/three"] {
                let root = &root;
                scope.spawn(move || {
                    update(
                        root,
                        folder,
                        Change::Update {
                            added: paths(&[folder]),
                            removed: paths(&[]),
                        },
                    )
                    .unwrap()
                });
            }
        });
        let reopened = update(&root, "new", Change::Initialize { paths: paths(&[]) }).unwrap();
        assert_eq!(reopened.paths, paths(&["/one", "/two", "/three"]));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bridge_accepts_only_its_own_origin_token_and_message_schema() {
        let origin = tauri::Url::parse("http://127.0.0.1:53172").unwrap();
        let bridge = Bridge::new(&origin, "one", "editor-one").unwrap();
        let mut target = origin.join(BRIDGE_PATH).unwrap();
        target
            .query_pairs_mut()
            .append_pair("token", &bridge.token)
            .append_pair(
                "message",
            r#"{"id":1,"documentId":"document-one","change":{"operation":"initialize","paths":["/repos"]}}"#,
            );
        assert_eq!(bridge.request(&target).unwrap().id, 1);
        let other = Bridge::new(&origin, "two", "editor-two").unwrap();
        assert!(other.request(&target).is_none());
        target.set_port(Some(53173)).unwrap();
        assert!(bridge.request(&target).is_none());
        target.set_port(Some(53172)).unwrap();
        target.set_path("/other");
        assert!(bridge.request(&target).is_none());
        target.set_path(BRIDGE_PATH);
        target.query_pairs_mut().append_pair(
            "message",
            r#"{"id":1,"change":{"operation":"execute","command":"anything"}}"#,
        );
        assert!(bridge.request(&target).is_none());
    }
}
