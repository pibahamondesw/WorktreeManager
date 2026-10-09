#[cfg(target_os = "macos")]
#[path = "../src/menu.rs"]
#[allow(dead_code, unused_imports)]
mod menu;

#[cfg(target_os = "macos")]
#[path = "../src/commands/mod.rs"]
#[allow(dead_code, unused_imports)]
mod commands;

#[cfg(target_os = "macos")]
use tauri::{
    menu::{Menu, MenuItemKind, PredefinedMenuItem},
    Manager,
};

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}

#[cfg(target_os = "macos")]
unsafe impl objc2::encode::Encode for Point {
    const ENCODING: objc2::encode::Encoding = objc2::encode::Encoding::Struct(
        "CGPoint",
        &[
            objc2::encode::Encoding::Double,
            objc2::encode::Encoding::Double,
        ],
    );
}

#[cfg(target_os = "macos")]
fn key_event(characters: &str, modifiers: usize, key_code: u16) -> *mut objc2::runtime::AnyObject {
    use objc2::{msg_send, runtime::AnyClass, runtime::AnyObject};

    let characters = std::ffi::CString::new(characters).unwrap();
    unsafe {
        let characters: *mut AnyObject = msg_send![
            AnyClass::get(c"NSString").unwrap(),
            stringWithUTF8String: characters.as_ptr()
        ];
        msg_send![
            AnyClass::get(c"NSEvent").unwrap(),
            keyEventWithType: 10usize,
            location: Point { x: 0.0, y: 0.0 },
            modifierFlags: modifiers,
            timestamp: 0.0f64,
            windowNumber: 0isize,
            context: std::ptr::null_mut::<AnyObject>(),
            characters: characters,
            charactersIgnoringModifiers: characters,
            isARepeat: false,
            keyCode: key_code
        ]
    }
}

#[cfg(target_os = "macos")]
fn wait_title(view: &tauri::Webview, expected: &str) {
    use objc2::{msg_send, runtime::AnyClass, runtime::AnyObject};

    let title = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let start = std::time::Instant::now();
    objc2::rc::autoreleasepool(|_| unsafe {
        loop {
            let current_title = title.clone();
            view.with_webview(move |platform| {
                let view = &*platform.inner().cast::<AnyObject>();
                let title: *mut AnyObject = msg_send![view, title];
                if let Some(title) = title.as_ref() {
                    let utf8: *const std::ffi::c_char = msg_send![title, UTF8String];
                    if !utf8.is_null() {
                        *current_title.lock().unwrap() = std::ffi::CStr::from_ptr(utf8)
                            .to_string_lossy()
                            .into_owned();
                    }
                }
            })
            .unwrap();
            if *title.lock().unwrap() == expected {
                return;
            }
            assert!(
                start.elapsed().as_secs() < 5,
                "WebKit did not report {expected:?}"
            );
            let run_loop: *mut AnyObject =
                msg_send![AnyClass::get(c"NSRunLoop").unwrap(), currentRunLoop];
            let date: *mut AnyObject = msg_send![
                AnyClass::get(c"NSDate").unwrap(),
                dateWithTimeIntervalSinceNow: 0.01f64
            ];
            let _: () = msg_send![run_loop, runUntilDate: date];
        }
    });
}

#[cfg(target_os = "macos")]
fn assert_key_routing(
    view: &tauri::Webview,
    key: &str,
    modifiers: usize,
    code: u16,
    expected: bool,
) {
    let key = key.to_owned();
    view.with_webview(move |platform| unsafe {
        let view = &*platform.inner().cast::<objc2::runtime::AnyObject>();
        objc2::rc::autoreleasepool(|_| {
            let event = key_event(&key, modifiers, code);
            let handled: bool = objc2::msg_send![view, performKeyEquivalent: event];
            assert_eq!(handled, expected, "incorrect routing for {key:?}");
        });
    })
    .unwrap();
}

#[cfg(target_os = "macos")]
fn assert_non_text_events_have_no_priority(view: &tauri::Webview) {
    view.with_webview(|_| unsafe {
        use objc2::{msg_send, runtime::AnyClass, runtime::AnyObject};

        objc2::rc::autoreleasepool(|_| {
            let characters = [0xd800u16];
            let characters: *mut AnyObject = msg_send![
                AnyClass::get(c"NSString").unwrap(),
                stringWithCharacters: characters.as_ptr(),
                length: 1usize
            ];
            let utf8: *const std::ffi::c_char = msg_send![characters, UTF8String];
            assert!(utf8.is_null());
            let event: *mut AnyObject = msg_send![
                AnyClass::get(c"NSEvent").unwrap(),
                keyEventWithType: 10usize,
                location: Point { x: 0.0, y: 0.0 },
                modifierFlags: 1usize << 20,
                timestamp: 0.0f64,
                windowNumber: 0isize,
                context: std::ptr::null_mut::<AnyObject>(),
                characters: characters,
                charactersIgnoringModifiers: characters,
                isARepeat: false,
                keyCode: 55u16
            ];
            menu::editor::native_test_nontext_event(event);
            let mut class = objc2::runtime::ClassBuilder::new(
                c"WTMNonTextEvent",
                AnyClass::get(c"NSObject").unwrap(),
            )
            .unwrap();
            class.add_method(
                objc2::sel!(charactersIgnoringModifiers),
                no_characters
                    as unsafe extern "C-unwind" fn(
                        *mut AnyObject,
                        objc2::runtime::Sel,
                    ) -> *mut AnyObject,
            );
            class.add_method(
                objc2::sel!(modifierFlags),
                no_modifiers
                    as unsafe extern "C-unwind" fn(*mut AnyObject, objc2::runtime::Sel) -> usize,
            );
            let event: *mut AnyObject = msg_send![class.register(), new];
            menu::editor::native_test_nontext_event(event);
            let _: () = msg_send![event, release];
        });
    })
    .unwrap();
}

#[cfg(target_os = "macos")]
unsafe extern "C-unwind" fn no_characters(
    _: *mut objc2::runtime::AnyObject,
    _: objc2::runtime::Sel,
) -> *mut objc2::runtime::AnyObject {
    std::ptr::null_mut()
}

#[cfg(target_os = "macos")]
unsafe extern "C-unwind" fn no_modifiers(
    _: *mut objc2::runtime::AnyObject,
    _: objc2::runtime::Sel,
) -> usize {
    0
}

#[cfg(target_os = "macos")]
fn assert_detached_editor_does_not_handle_keys(view: &tauri::Webview) {
    view.with_webview(|platform| unsafe {
        use objc2::{msg_send, runtime::AnyObject};

        let view = &*platform.inner().cast::<AnyObject>();
        let parent: *mut AnyObject = msg_send![view, superview];
        let _: () = msg_send![view, removeFromSuperview];
        objc2::rc::autoreleasepool(|_| {
            let event = key_event("z", 1 << 20, 6);
            let handled: bool = msg_send![view, performKeyEquivalent: event];
            assert!(!handled);
        });
        let _: () = msg_send![parent, addSubview: view];
    })
    .unwrap();
}

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

    let window = tauri::window::WindowBuilder::new(&app, "main")
        .visible(false)
        .build()
        .unwrap();
    let create_view = |label| {
        window
            .add_child(
                tauri::webview::WebviewBuilder::new(
                    label,
                    tauri::WebviewUrl::External("about:blank".parse().unwrap()),
                ),
                tauri::LogicalPosition::new(0.0, 0.0),
                tauri::LogicalSize::new(100.0, 100.0),
            )
            .unwrap()
    };
    let main = create_view("shortcut-main");
    let created_editor = commands::code_server::native_test_view(app.handle(), "shortcut-editor");
    let editor = created_editor.webview.clone();
    editor.show().unwrap();
    editor.set_focus().unwrap();
    editor
        .with_webview(|platform| unsafe {
            use objc2::{msg_send, runtime::AnyClass, runtime::AnyObject};
            let view = &*platform.inner().cast::<AnyObject>();
            let html: *mut AnyObject = msg_send![
                AnyClass::get(c"NSString").unwrap(),
                stringWithUTF8String: c"<title>ready</title><input autofocus>".as_ptr()
            ];
            let _: *mut AnyObject =
                msg_send![view, loadHTMLString: html, baseURL: std::ptr::null_mut::<AnyObject>()];
        })
        .unwrap();
    wait_title(&editor, "ready");
    editor.eval("document.addEventListener('keydown', (event) => { event.preventDefault(); document.title = 'key:' + event.key; }); document.title = 'listening';").unwrap();
    wait_title(&editor, "listening");
    menu::set_shortcut_context(app.handle().clone(), "task".into(), false, app.state()).unwrap();
    assert_key_routing(&main, "z", 1 << 20, 6, false);
    assert_key_routing(&editor, "k", 1 << 20, 40, false);
    for (key, modifiers, key_code, expected) in [
        ("z", 1 << 20, 6, "key:z"),
        ("Z", (1 << 20) | (1 << 17), 6, "key:Z"),
        ("\u{f702}", 1 << 20, 123, "key:ArrowLeft"),
        ("\u{f703}", 1 << 20, 124, "key:ArrowRight"),
        ("[", 1 << 20, 33, "key:["),
        ("p", (1 << 20) | (1 << 19), 35, "key:p"),
    ] {
        assert_key_routing(&editor, key, modifiers, key_code, true);
        wait_title(&editor, expected);
    }
    let rebound = serde_json::from_value(serde_json::json!([
        {"id": "app.search.0", "action": "search", "key": "y", "modifiers": ["meta"], "contexts": ["task"], "inText": true}
    ])).unwrap();
    menu::set_shortcut_menu(app.handle().clone(), rebound, app.state()).unwrap();
    assert_key_routing(&editor, "y", 1 << 20, 16, false);
    assert_key_routing(&editor, "k", 1 << 20, 40, true);
    wait_title(&editor, "key:k");
    menu::set_shortcut_capture(app.handle().clone(), true, app.state()).unwrap();
    assert_key_routing(&editor, "y", 1 << 20, 16, true);
    wait_title(&editor, "key:y");
    menu::set_shortcut_capture(app.handle().clone(), false, app.state()).unwrap();
    assert_key_routing(&editor, "y", 1 << 20, 16, false);
    menu::set_shortcut_menu(app.handle().clone(), vec![], app.state()).unwrap();
    assert_key_routing(&editor, "y", 1 << 20, 16, true);
    main.set_focus().unwrap();
    assert_key_routing(&editor, "z", 1 << 20, 6, false);
    editor.set_focus().unwrap();
    editor.hide().unwrap();
    assert_key_routing(&editor, "z", 1 << 20, 6, false);
    editor.show().unwrap();
    editor.set_focus().unwrap();
    assert_non_text_events_have_no_priority(&editor);
    assert_detached_editor_does_not_handle_keys(&editor);
    drop(created_editor);
    assert_key_routing(&editor, "z", 1 << 20, 6, false);
    commands::code_server::native_test_failed_view_cleanup(&editor);
    main.close().unwrap();
    window.close().unwrap();
    println!("menu regression passed: no native close command; other actions preserved");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("menu regression skipped: requires the macOS native menu");
}
