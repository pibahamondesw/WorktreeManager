use std::collections::BTreeMap;
use std::ffi::CStr;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use objc2::runtime::{AnyClass, AnyObject, Bool, Sel};
use objc2::{msg_send, sel};

use super::ShortcutBinding;

const SHIFT: usize = 1 << 17;
const CONTROL: usize = 1 << 18;
const OPTION: usize = 1 << 19;
const COMMAND: usize = 1 << 20;
const MODIFIERS: usize = SHIFT | CONTROL | OPTION | COMMAND;
static PRIORITY_BINDINGS: Mutex<Vec<ShortcutBinding>> = Mutex::new(Vec::new());
static EDITOR_VIEWS: Mutex<BTreeMap<usize, Weak<()>>> = Mutex::new(BTreeMap::new());
type KeyEquivalent = unsafe extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> Bool;
static ORIGINAL_KEY_EQUIVALENT: OnceLock<KeyEquivalent> = OnceLock::new();

pub(super) fn update_priority(
    bindings: &[ShortcutBinding],
    capturing: bool,
    context: &str,
    editing: bool,
) {
    *PRIORITY_BINDINGS.lock().unwrap() = bindings
        .iter()
        .filter(|binding| {
            !capturing
                && binding.id.starts_with("app.search.")
                && binding.action == "search"
                && binding.contexts.iter().any(|value| value == context)
                && (!editing || binding.in_text)
        })
        .cloned()
        .collect();
}

fn matches(binding: &ShortcutBinding, characters: &str, flags: usize) -> bool {
    let key = match binding.key.as_str() {
        "ArrowUp" => "\u{f700}",
        "ArrowDown" => "\u{f701}",
        "ArrowLeft" => "\u{f702}",
        "ArrowRight" => "\u{f703}",
        "Enter" => "\r",
        "Tab" => "\t",
        "Escape" => "\u{1b}",
        "Backspace" => "\u{7f}",
        "Delete" => "\u{f728}",
        "Home" => "\u{f729}",
        "End" => "\u{f72b}",
        "PageUp" => "\u{f72c}",
        "PageDown" => "\u{f72d}",
        other => other,
    };
    let mut modifiers = 0;
    for modifier in &binding.modifiers {
        modifiers |= match modifier.as_str() {
            "meta" => COMMAND,
            "ctrl" => CONTROL,
            "alt" => OPTION,
            "shift" => SHIFT,
            _ => return false,
        };
    }
    if key == "+" {
        modifiers |= SHIFT;
    }
    let key_matches =
        if let Some(number) = key.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
            (1..=24).contains(&number) && characters.chars().eq(char::from_u32(0xf703 + number))
        } else {
            key.eq_ignore_ascii_case(characters)
        };
    key_matches && flags & MODIFIERS == modifiers
}

fn system_shortcut(characters: &str, flags: usize) -> bool {
    let modifiers = flags & MODIFIERS;
    match characters.to_ascii_lowercase().as_str() {
        "q" | "m" => modifiers == COMMAND,
        "h" => modifiers == COMMAND || modifiers == COMMAND | OPTION,
        "f" => modifiers == COMMAND | CONTROL,
        _ => false,
    }
}

fn focused(view: &AnyObject) -> bool {
    unsafe {
        let hidden: bool = msg_send![view, isHiddenOrHasHiddenAncestor];
        if hidden {
            return false;
        }
        let window: *mut AnyObject = msg_send![view, window];
        let responder: *mut AnyObject = msg_send![window, firstResponder];
        let Some(responder) = responder.as_ref() else {
            return false;
        };
        let is_view: bool = msg_send![responder, isKindOfClass: AnyClass::get(c"NSView").unwrap()];
        is_view && msg_send![responder, isDescendantOf: view]
    }
}

extern "C-unwind" fn perform_key_equivalent(
    view: &AnyObject,
    selector: Sel,
    event: *mut AnyObject,
) -> Bool {
    unsafe {
        let editor = EDITOR_VIEWS
            .lock()
            .unwrap()
            .get(&(view as *const AnyObject as usize))
            .is_some_and(|lease| lease.strong_count() > 0);
        if !editor || !focused(view) {
            return ORIGINAL_KEY_EQUIVALENT.get().unwrap()(view, selector, event);
        }
        let characters: *mut AnyObject = msg_send![event, charactersIgnoringModifiers];
        let flags: usize = msg_send![event, modifierFlags];
        if let Some(characters) = characters.as_ref() {
            let utf8: *const std::ffi::c_char = msg_send![characters, UTF8String];
            if !utf8.is_null() {
                let characters = CStr::from_ptr(utf8).to_string_lossy();
                if system_shortcut(&characters, flags)
                    || PRIORITY_BINDINGS
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|binding| matches(binding, &characters, flags))
                {
                    return Bool::NO;
                }
            }
        }
        let _: () = msg_send![view, keyDown: event];
        Bool::YES
    }
}

pub fn install(view: &tauri::Webview) -> Result<Arc<()>, String> {
    let lease = Arc::new(());
    let registration = Arc::downgrade(&lease);
    view.with_webview(move |platform| unsafe {
        ORIGINAL_KEY_EQUIVALENT.get_or_init(|| {
            let method = AnyClass::get(c"WryWebView")
                .unwrap()
                .instance_method(sel!(performKeyEquivalent:))
                .unwrap();
            let implementation =
                std::mem::transmute::<KeyEquivalent, objc2::runtime::Imp>(perform_key_equivalent);
            std::mem::transmute::<objc2::runtime::Imp, KeyEquivalent>(
                method.set_implementation(implementation),
            )
        });
        let view = &*platform.inner().cast::<AnyObject>();
        let mut views = EDITOR_VIEWS.lock().unwrap();
        views.retain(|_, lease| lease.strong_count() > 0);
        views.insert(view as *const AnyObject as usize, registration);
    })
    .map_err(|error| error.to_string())?;
    Ok(lease)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(key: &str, modifiers: &[&str]) -> ShortcutBinding {
        ShortcutBinding {
            id: "app.search.0".into(),
            action: "search".into(),
            key: key.into(),
            modifiers: modifiers.iter().map(|value| (*value).into()).collect(),
            contexts: vec!["task".into()],
            in_text: true,
        }
    }

    #[test]
    fn matches_rebound_search_with_exact_modifiers_and_ignores_caps_lock() {
        let search = binding("y", &["meta", "alt"]);
        assert!(matches(&search, "Y", COMMAND | OPTION | (1 << 16)));
        assert!(!matches(&search, "k", COMMAND));
        assert!(!matches(&search, "y", COMMAND));
        assert!(!matches(&search, "y", COMMAND | OPTION | SHIFT));
        assert!(!matches(&search, "y", COMMAND | OPTION | CONTROL));
    }

    #[test]
    fn matches_special_keys_and_symbols() {
        for (key, characters) in [
            ("ArrowLeft", "\u{f702}"),
            ("Enter", "\r"),
            ("F1", "\u{f704}"),
            ("F24", "\u{f71b}"),
            ("[", "["),
        ] {
            assert!(matches(&binding(key, &["meta"]), characters, COMMAND));
        }
        assert!(matches(&binding("+", &["meta"]), "+", COMMAND | SHIFT));
    }

    #[test]
    fn prioritizes_only_global_search_and_tracks_menu_changes() {
        let search = binding("k", &["meta"]);
        let mut alternate = binding("p", &["meta", "alt"]);
        alternate.id = "native.search.0".into();
        let bindings = [search, alternate];
        let priority = |key, flags| {
            PRIORITY_BINDINGS
                .lock()
                .unwrap()
                .iter()
                .any(|binding| matches(binding, key, flags))
        };
        update_priority(&bindings, false, "task", true);
        assert!(priority("k", COMMAND));
        assert!(!priority("p", COMMAND | OPTION));
        for key in ["z", "[", "\u{f702}", "\u{f703}"] {
            assert!(!priority(key, COMMAND));
        }
        let rebound = [binding("y", &["meta"])];
        update_priority(&rebound, false, "task", true);
        assert!(priority("y", COMMAND));
        assert!(!priority("k", COMMAND));
        update_priority(&rebound, true, "task", true);
        assert!(!priority("y", COMMAND));
        update_priority(&rebound, false, "modal", true);
        assert!(!priority("y", COMMAND));
        update_priority(&[], false, "task", true);
        assert!(!priority("y", COMMAND));
    }

    #[test]
    fn preserves_system_shortcuts_without_claiming_editor_commands() {
        assert!(system_shortcut("q", COMMAND));
        assert!(system_shortcut("h", COMMAND | OPTION));
        assert!(system_shortcut("m", COMMAND));
        assert!(system_shortcut("f", COMMAND | CONTROL));
        for key in ["z", "k", "p", "[", "\u{f702}", "\u{f703}"] {
            assert!(!system_shortcut(key, COMMAND));
        }
    }
}
