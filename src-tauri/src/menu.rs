use tauri::{Emitter, Manager};

pub fn setup(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
    let menu = Menu::default(app.handle())?;
    let close_window_text = PredefinedMenuItem::close_window(app, None)?.text()?;
    for item in menu.items()? {
        if let Some(submenu) = item.as_submenu() {
            for item in submenu.items()? {
                if let Some(predefined) = item.as_predefined_menuitem() {
                    if predefined.text()? == close_window_text {
                        submenu.remove(predefined)?;
                    }
                }
            }
        }
    }
    let back = MenuItem::with_id(
        app,
        "editor-back",
        "Back to tasks",
        true,
        Some("CmdOrCtrl+["),
    )?;
    let search = MenuItem::with_id(
        app,
        "editor-search",
        "Search tasks",
        true,
        Some("CmdOrCtrl+Alt+P"),
    )?;
    menu.append(&Submenu::with_items(app, "Tasks", true, &[&back, &search])?)?;
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        let action = match event.id().as_ref() {
            "editor-back" => "back",
            "editor-search" => "search",
            _ => return,
        };
        if let Some(main) = app.get_webview("main") {
            let _ = main.set_focus();
        }
        let _ = app.emit_to("main", "editor-navigate", action);
    });
    Ok(())
}
