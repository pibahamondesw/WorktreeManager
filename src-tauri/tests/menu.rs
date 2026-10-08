#[cfg(target_os = "macos")]
#[path = "../src/menu.rs"]
mod menu;

#[cfg(target_os = "macos")]
use tauri::{
    menu::{Menu, MenuItemKind, PredefinedMenuItem},
    Manager,
};

#[cfg(target_os = "macos")]
fn predefined_items(items: Vec<MenuItemKind<tauri::Wry>>) -> Vec<String> {
    let mut names = Vec::new();
    for item in items {
        match item {
            MenuItemKind::Submenu(submenu) => {
                names.extend(predefined_items(submenu.items().unwrap()));
            }
            MenuItemKind::Predefined(item) => names.push(item.text().unwrap()),
            _ => {}
        }
    }
    names
}

#[cfg(target_os = "macos")]
fn main() {
    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .unwrap();
    let default_menu = Menu::default(app.handle()).unwrap();
    let close_text = PredefinedMenuItem::close_window(&app, None)
        .unwrap()
        .text()
        .unwrap();
    let mut expected = predefined_items(default_menu.items().unwrap());
    assert!(expected.contains(&close_text));
    expected.retain(|text| text != &close_text);

    menu::setup(&app).unwrap();

    let menu = app.menu().unwrap();
    assert_eq!(predefined_items(menu.items().unwrap()), expected);
    let items = menu.items().unwrap();
    let tasks = items.last().unwrap().as_submenu().unwrap();
    for action in ["back", "search", "history-back", "history-forward"] {
        assert!(tasks
            .get(&format!("shortcut:{action}:unassigned"))
            .is_some());
    }

    let bindings = serde_json::from_value(serde_json::json!([
        {"id": "native.back.0", "action": "back", "key": "[", "modifiers": ["meta"], "contexts": ["list", "task"], "inText": true},
        {"id": "app.search.0", "action": "search", "key": "k", "modifiers": ["meta"], "contexts": ["list", "task"], "inText": true},
        {"id": "app.history.back.0", "action": "history-back", "key": "ArrowLeft", "modifiers": ["meta"], "contexts": ["list", "task"], "inText": false},
        {"id": "app.history.forward.0", "action": "history-forward", "key": "ArrowRight", "modifiers": ["meta"], "contexts": ["list", "task"], "inText": false}
    ])).unwrap();
    menu::set_shortcut_menu(app.handle().clone(), bindings, app.state()).unwrap();
    for capturing in [false, true, false] {
        menu::set_shortcut_capture(app.handle().clone(), capturing, app.state()).unwrap();
        let menu = app.menu().unwrap();
        let items = menu.items().unwrap();
        let tasks = items.last().unwrap().as_submenu().unwrap();
        for id in [
            "shortcut:back:native.back.0",
            "shortcut:search:app.search.0",
            "shortcut:history-back:app.history.back.0",
            "shortcut:history-forward:app.history.forward.0",
        ] {
            assert_eq!(
                tasks
                    .get(id)
                    .unwrap()
                    .as_menuitem()
                    .unwrap()
                    .is_enabled()
                    .unwrap(),
                !capturing
            );
        }
        if !capturing {
            assert_eq!(predefined_items(items), expected);
        }
    }
    menu::set_shortcut_context(app.handle().clone(), "list".into(), true, app.state()).unwrap();
    let menu = app.menu().unwrap();
    let items = menu.items().unwrap();
    let tasks = items.last().unwrap().as_submenu().unwrap();
    assert!(tasks
        .get("shortcut:history-back:app.history.back.0")
        .is_none());
    assert!(tasks
        .get("shortcut:history-forward:app.history.forward.0")
        .is_none());
    assert!(tasks.get("shortcut:search:app.search.0").is_some());
    println!("menu regression passed: no native close command; other actions preserved");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("menu regression skipped: requires the macOS native menu");
}
