use serde::Deserialize;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutBinding {
    id: String,
    action: String,
    key: String,
    modifiers: Vec<String>,
    contexts: Vec<String>,
    #[serde(default)]
    in_text: bool,
}

#[derive(Default)]
pub struct ShortcutMenuState {
    bindings: Vec<ShortcutBinding>,
    capturing: bool,
    context: String,
    editing: bool,
}

fn accelerator(binding: &ShortcutBinding) -> Result<String, String> {
    if !matches!(
        binding.action.as_str(),
        "back" | "search" | "history-back" | "history-forward"
    ) {
        return Err("Unknown shortcut command".into());
    }
    if binding.modifiers.is_empty() {
        return Err("Native shortcuts require a modifier".into());
    }
    let mut parts = Vec::new();
    for modifier in &binding.modifiers {
        parts.push(match modifier.as_str() {
            "meta" => "Super",
            "ctrl" => "Control",
            "alt" => "Alt",
            "shift" => "Shift",
            _ => return Err("Unknown shortcut modifier".into()),
        });
    }
    if binding.key == "+" && !binding.modifiers.iter().any(|m| m == "shift") {
        parts.push("Shift");
    }
    let key = match binding.key.as_str() {
        "+" => "Equal",
        " " => "Space",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "ArrowUp" => "Up",
        "ArrowDown" => "Down",
        other => other,
    };
    parts.push(key);
    Ok(parts.join("+"))
}

fn apply_menu(
    app: &tauri::AppHandle,
    bindings: &[ShortcutBinding],
    capturing: bool,
    context: &str,
    editing: bool,
) -> Result<(), String> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
    let menu = if capturing {
        Menu::new(app)
    } else {
        Menu::default(app)
    }
    .map_err(|e| e.to_string())?;
    let close_window_text = PredefinedMenuItem::close_window(app, None)
        .and_then(|item| item.text())
        .map_err(|e| e.to_string())?;
    for item in menu.items().map_err(|e| e.to_string())? {
        if let Some(submenu) = item.as_submenu() {
            for item in submenu.items().map_err(|e| e.to_string())? {
                if let Some(predefined) = item.as_predefined_menuitem() {
                    if predefined.text().map_err(|e| e.to_string())? == close_window_text {
                        submenu.remove(predefined).map_err(|e| e.to_string())?;
                    }
                }
            }
        }
    }
    let tasks = Submenu::new(app, "Tasks", true).map_err(|e| e.to_string())?;
    for (action, label) in [
        ("back", "Back to tasks"),
        ("search", "Search tasks"),
        ("history-back", "Previous location"),
        ("history-forward", "Next location"),
    ] {
        let values: Vec<_> = bindings
            .iter()
            .filter(|b| {
                b.action == action
                    && b.contexts.iter().any(|c| c == context)
                    && (!editing || b.in_text)
            })
            .collect();
        if values.is_empty() {
            let item = MenuItem::with_id(
                app,
                format!("shortcut:{action}:unassigned"),
                label,
                !capturing,
                None::<&str>,
            )
            .map_err(|e| e.to_string())?;
            tasks.append(&item).map_err(|e| e.to_string())?;
        }
        for binding in values {
            let accel = accelerator(binding)?;
            let item = MenuItem::with_id(
                app,
                format!("shortcut:{action}:{}", binding.id),
                label,
                !capturing,
                if capturing {
                    None
                } else {
                    Some(accel.as_str())
                },
            )
            .map_err(|e| format!("Cannot assign native shortcut: {e}"))?;
            tasks.append(&item).map_err(|e| e.to_string())?;
        }
    }
    menu.append(&tasks).map_err(|e| e.to_string())?;
    app.set_menu(menu).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn set_shortcut_menu(
    app: tauri::AppHandle,
    bindings: Vec<ShortcutBinding>,
    state: tauri::State<'_, Mutex<ShortcutMenuState>>,
) -> Result<(), String> {
    for binding in &bindings {
        accelerator(binding)?;
    }
    let mut state = state.lock().map_err(|e| e.to_string())?;
    if state.capturing {
        for binding in &bindings {
            tauri::menu::MenuItem::with_id(
                &app,
                format!("validate:{}", binding.id),
                "",
                false,
                Some(accelerator(binding)?),
            )
            .map_err(|e| format!("Cannot assign native shortcut: {e}"))?;
        }
    }
    apply_menu(
        &app,
        &bindings,
        state.capturing,
        &state.context,
        state.editing,
    )?;
    state.bindings = bindings;
    Ok(())
}

#[tauri::command]
pub fn set_shortcut_capture(
    app: tauri::AppHandle,
    active: bool,
    state: tauri::State<'_, Mutex<ShortcutMenuState>>,
) -> Result<(), String> {
    let mut state = state.lock().map_err(|e| e.to_string())?;
    apply_menu(&app, &state.bindings, active, &state.context, state.editing)?;
    state.capturing = active;
    Ok(())
}

#[tauri::command]
pub fn set_shortcut_context(
    app: tauri::AppHandle,
    context: String,
    editing: bool,
    state: tauri::State<'_, Mutex<ShortcutMenuState>>,
) -> Result<(), String> {
    if !matches!(
        context.as_str(),
        "list" | "task" | "chat" | "palette" | "modal"
    ) {
        return Err("Unknown shortcut context".into());
    }
    let mut state = state.lock().map_err(|e| e.to_string())?;
    apply_menu(&app, &state.bindings, state.capturing, &context, editing)?;
    state.context = context;
    state.editing = editing;
    Ok(())
}

pub fn setup(app: &tauri::App) -> tauri::Result<()> {
    app.manage(Mutex::new(ShortcutMenuState {
        context: "list".into(),
        ..Default::default()
    }));
    apply_menu(app.handle(), &[], false, "list", false).map_err(std::io::Error::other)?;
    app.on_menu_event(|app, event| {
        let id = event.id().as_ref();
        let action = if id.starts_with("shortcut:back:") {
            "back"
        } else if id.starts_with("shortcut:search:") {
            "search"
        } else if id.starts_with("shortcut:history-back:") {
            "history-back"
        } else if id.starts_with("shortcut:history-forward:") {
            "history-forward"
        } else {
            return;
        };
        if let Some(main) = app.get_webview("main") {
            let _ = main.set_focus();
        }
        let _ = app.emit_to("main", "editor-navigate", action);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn translates_explicit_modifiers_and_symbols() {
        let binding = ShortcutBinding {
            id: "native.back.0".into(),
            action: "back".into(),
            key: "[".into(),
            modifiers: vec!["meta".into()],
            contexts: vec!["task".into()],
            in_text: false,
        };
        assert_eq!(accelerator(&binding).unwrap(), "Super+[");
        assert_eq!(
            accelerator(&ShortcutBinding {
                key: "+".into(),
                ..binding
            })
            .unwrap(),
            "Super+Shift+Equal"
        );
    }
    #[test]
    fn translates_history_accelerators() {
        for (action, key, expected) in [
            ("history-back", "ArrowLeft", "Super+Left"),
            ("history-forward", "ArrowRight", "Super+Right"),
        ] {
            let binding = ShortcutBinding {
                id: action.into(),
                action: action.into(),
                key: key.into(),
                modifiers: vec!["meta".into()],
                contexts: vec!["list".into()],
                in_text: false,
            };
            assert_eq!(accelerator(&binding).unwrap(), expected);
        }
    }

    #[test]
    fn rejects_unknown_commands_and_modifiers() {
        let mut binding = ShortcutBinding {
            id: "x".into(),
            action: "other".into(),
            key: "p".into(),
            modifiers: vec!["meta".into()],
            contexts: vec!["task".into()],
            in_text: false,
        };
        assert!(accelerator(&binding).is_err());
        binding.action = "search".into();
        binding.modifiers = vec!["unknown".into()];
        assert!(accelerator(&binding).is_err());
    }
}
