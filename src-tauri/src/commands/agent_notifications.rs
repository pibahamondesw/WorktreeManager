//! System notifications for agent alerts: at most one per task, delivered without waiting for the
//! user. Clicks reach the app through a notification center delegate, and a task's notification is
//! replaced by its next alert and removed once the task no longer needs attention.

use super::agent_alerts::NotificationTarget;

const IDENTIFIER_PREFIX: &str = "wtm-agent-task:";
const TARGET_KEY: &str = "target";

fn task_identifier(task_id: &str) -> String {
    format!("{IDENTIFIER_PREFIX}{task_id}")
}

fn notifies_task(identifier: &str, task_ids: &[String]) -> bool {
    identifier
        .strip_prefix(IDENTIFIER_PREFIX)
        .is_some_and(|task_id| task_ids.iter().any(|id| id == task_id))
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
mod macos {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
    use objc2_foundation::{
        NSDictionary, NSString, NSUserNotification, NSUserNotificationCenter,
        NSUserNotificationCenterDelegate,
    };
    use tauri::AppHandle;

    use super::{notifies_task, task_identifier, NotificationTarget, TARGET_KEY};
    use crate::commands::agent_alerts::reveal_target;

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "WTMAgentNotificationDelegate"]
        #[ivars = AppHandle]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl NSUserNotificationCenterDelegate for Delegate {
            #[unsafe(method(userNotificationCenter:shouldPresentNotification:))]
            fn should_present(
                &self,
                _center: &NSUserNotificationCenter,
                _notification: &NSUserNotification,
            ) -> bool {
                true
            }

            #[unsafe(method(userNotificationCenter:didActivateNotification:))]
            fn did_activate(
                &self,
                center: &NSUserNotificationCenter,
                notification: &NSUserNotification,
            ) {
                if let Some(target) = stored_target(notification) {
                    reveal_target(self.ivars(), &target);
                }
                center.removeDeliveredNotification(notification);
            }
        }
    );

    fn stored_target(notification: &NSUserNotification) -> Option<NotificationTarget> {
        let json = notification
            .userInfo()?
            .objectForKey(&NSString::from_str(TARGET_KEY))?
            .downcast::<NSString>()
            .ok()?
            .to_string();
        serde_json::from_str(&json).ok()
    }

    fn center() -> Retained<NSUserNotificationCenter> {
        NSUserNotificationCenter::defaultUserNotificationCenter()
    }

    /// Must run on the main thread, before the first notification, so clicks on notifications left
    /// from a previous launch also reach the app. Unbundled dev builds borrow the app's bundle
    /// identifier, without which the notification center is unavailable.
    pub fn install(app: &AppHandle) {
        let _ = mac_notification_sys::set_application(&app.config().identifier);
        let delegate = Delegate::alloc().set_ivars(app.clone());
        let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
        unsafe { center().setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        std::mem::forget(delegate);
    }

    fn remove_for_tasks(center: &NSUserNotificationCenter, task_ids: &[String]) {
        for notification in center.deliveredNotifications() {
            let identifier = notification.identifier().map(|id| id.to_string());
            if identifier.is_some_and(|id| notifies_task(&id, task_ids)) {
                center.removeDeliveredNotification(&notification);
            }
        }
    }

    pub fn deliver(app: &AppHandle, title: String, body: String, target: NotificationTarget) {
        let _ = app.run_on_main_thread(move || {
            let Ok(target_json) = serde_json::to_string(&target) else {
                return;
            };
            let center = center();
            remove_for_tasks(&center, std::slice::from_ref(&target.task_id));
            let notification = NSUserNotification::new();
            notification
                .setIdentifier(Some(&NSString::from_str(&task_identifier(&target.task_id))));
            notification.setTitle(Some(&NSString::from_str(&title)));
            notification.setInformativeText(Some(&NSString::from_str(&body)));
            let target_value = NSString::from_str(&target_json);
            let user_info = NSDictionary::<NSString, AnyObject>::from_slices(
                &[&*NSString::from_str(TARGET_KEY)],
                &[&**target_value],
            );
            unsafe { notification.setUserInfo(Some(&user_info)) };
            center.deliverNotification(&notification);
        });
    }

    pub fn dismiss(app: &AppHandle, task_ids: Vec<String>) {
        let _ = app.run_on_main_thread(move || remove_for_tasks(&center(), &task_ids));
    }
}

#[cfg(target_os = "macos")]
pub use macos::{deliver, dismiss, install};

#[cfg(not(target_os = "macos"))]
pub fn install(_app: &tauri::AppHandle) {}

#[cfg(not(target_os = "macos"))]
pub fn deliver(
    _app: &tauri::AppHandle,
    _title: String,
    _body: String,
    _target: NotificationTarget,
) {
}

#[cfg(not(target_os = "macos"))]
pub fn dismiss(_app: &tauri::AppHandle, _task_ids: Vec<String>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_match_only_their_own_tasks() {
        let ids = vec!["task-1".to_string(), "task-2".to_string()];
        assert!(notifies_task(&task_identifier("task-1"), &ids));
        assert!(notifies_task(&task_identifier("task-2"), &ids));
        assert!(!notifies_task(&task_identifier("task-10"), &ids));
        assert!(!notifies_task("task-1", &ids));
        assert!(!notifies_task("6F9619FF-8B86-D011-B42D-00C04FC964FF", &ids));
    }
}
