//! The app theme reaches the editor as default settings, below the user's and the workspace's,
//! so a theme picked inside the editor wins and its settings files are never written.

use std::sync::Mutex;

use serde_json::{Map, Value};
use tauri::{AppHandle, Manager, Runtime, State, Webview};

use crate::commands::embedded::LABEL_ROOT;

#[derive(Default)]
pub struct EditorTheme(Mutex<Map<String, Value>>);

impl EditorTheme {
    /// Applies the current defaults to a loaded workbench, or leaves them for the one starting.
    pub fn script(&self) -> String {
        let defaults = Value::Object(self.0.lock().unwrap().clone());
        format!(
            "globalThis.__wtmConfigurationDefaults = {defaults}; globalThis.__wtmApplyConfigurationDefaults?.({defaults});"
        )
    }

    fn follow<R: Runtime>(&self, app: &AppHandle<R>, defaults: Map<String, Value>) {
        *self.0.lock().unwrap() = defaults;
        let script = self.script();
        for (label, view) in app.webviews() {
            if label.starts_with(LABEL_ROOT) {
                let _ = view.eval(&script);
            }
        }
    }
}

/// Reapplies the latest defaults after the editor reloads, since its startup script keeps the
/// ones it was created with.
pub fn reapply<R: Runtime>(view: &Webview<R>) {
    if let Some(theme) = view.try_state::<EditorTheme>() {
        let _ = view.eval(theme.script());
    }
}

#[tauri::command]
pub fn vscode_set_theme<R: Runtime>(
    app: AppHandle<R>,
    theme: State<'_, EditorTheme>,
    defaults: Map<String, Value>,
) {
    theme.follow(&app, defaults);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime};
    use tauri::webview::InvokeRequest;

    fn app() -> tauri::App<MockRuntime> {
        mock_builder()
            .invoke_handler(tauri::generate_handler![vscode_set_theme])
            .manage(EditorTheme::default())
            .build(mock_context(noop_assets()))
            .unwrap()
    }

    fn set_theme(view: &tauri::WebviewWindow<MockRuntime>, defaults: Value) -> Result<(), Value> {
        get_ipc_response(
            view,
            InvokeRequest {
                cmd: "vscode_set_theme".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "http://tauri.localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::Json(json!({ "defaults": defaults })),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.to_string(),
            },
        )
        .map(|_| ())
    }

    #[test]
    fn a_new_editor_starts_without_app_defaults() {
        assert_eq!(
            EditorTheme::default().script(),
            "globalThis.__wtmConfigurationDefaults = {}; globalThis.__wtmApplyConfigurationDefaults?.({});"
        );
    }

    #[test]
    fn the_latest_app_theme_is_kept_for_editors_that_open_or_reload_later() {
        let app = app();
        let main = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let editor = tauri::WebviewWindowBuilder::new(&app, "editor-task", Default::default())
            .build()
            .unwrap();
        set_theme(&main, json!({"workbench.colorTheme": "Light Modern"})).unwrap();
        set_theme(&main, json!({"workbench.colorTheme": "Dark Modern"})).unwrap();
        let script = app.state::<EditorTheme>().script();
        assert!(script.contains(r#"{"workbench.colorTheme":"Dark Modern"}"#));
        assert!(!script.contains("Light"));
        reapply(editor.as_ref());
    }

    #[test]
    fn defaults_that_are_not_settings_are_rejected() {
        let app = app();
        let main = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        set_theme(&main, json!({"workbench.colorTheme": "Dark Modern"})).unwrap();
        assert!(set_theme(&main, json!(["workbench.colorTheme"])).is_err());
        assert!(app.state::<EditorTheme>().script().contains("Dark Modern"));
    }

    #[test]
    fn an_editor_without_the_theme_state_reloads_unchanged() {
        let app = mock_builder().build(mock_context(noop_assets())).unwrap();
        let editor = tauri::WebviewWindowBuilder::new(&app, "editor-task", Default::default())
            .build()
            .unwrap();
        reapply(editor.as_ref());
    }
}
