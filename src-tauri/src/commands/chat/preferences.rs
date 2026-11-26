use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::{Store, StoreExt};

use super::model::ChatControls;

static SAVE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Deserialize, Serialize)]
pub(crate) struct ModelSelection {
    pub model: Option<String>,
    pub effort: Option<String>,
}

fn store<R: Runtime>(app: &AppHandle<R>) -> Result<Arc<Store<R>>, String> {
    app.store_builder("chat-preferences.json")
        .disable_auto_save()
        .build()
        .map_err(|e| e.to_string())
}

pub(super) fn load<R: Runtime>(
    app: &AppHandle<R>,
    agent: &str,
) -> Result<Option<ModelSelection>, String> {
    let store = store(app)?;
    Ok(store
        .get(agent)
        .and_then(|value| serde_json::from_value::<ModelSelection>(value).ok())
        .filter(|selection| {
            selection
                .model
                .as_ref()
                .is_some_and(|model| !model.trim().is_empty())
        }))
}

pub(super) fn save<R: Runtime>(
    app: &AppHandle<R>,
    agent: &str,
    controls: &ChatControls,
) -> Result<(), String> {
    let _saving = SAVE_LOCK.lock().unwrap();
    let store = store(app)?;
    let selection = ModelSelection {
        model: controls.model.clone(),
        effort: controls.effort.clone(),
    };
    let previous = store.get(agent);
    store.set(
        agent,
        serde_json::to_value(selection).map_err(|e| e.to_string())?,
    );
    if let Err(error) = store.save() {
        if let Some(previous) = previous {
            store.set(agent, previous);
        } else {
            store.delete(agent);
        }
        return Err(error.to_string());
    }
    Ok(())
}
