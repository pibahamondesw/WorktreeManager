#[cfg(target_os = "macos")]
#[path = "../src/menu.rs"]
mod menu;

#[cfg(target_os = "macos")]
use tauri::menu::{Menu, MenuItemKind, PredefinedMenuItem};

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
    assert!(tasks.get("editor-back").is_some());
    assert!(tasks.get("editor-search").is_some());
    println!("menu regression passed: no native close command; other actions preserved");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("menu regression skipped: requires the macOS native menu");
}
