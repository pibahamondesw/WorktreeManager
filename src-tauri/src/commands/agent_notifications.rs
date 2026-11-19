//! System notifications for agent alerts: at most one per task, and one per automation Inbox
//! entry, delivered without waiting for the user. Clicks reach the app through a notification
//! center delegate, and a task's notification is replaced by its next alert and removed once the
//! task no longer needs attention.

use tauri::{AppHandle, Runtime};

use super::agent_alerts::NotificationTarget;

#[cfg(target_os = "macos")]
const IDENTIFIER_PREFIX: &str = "wtm-agent-task:";
#[cfg(target_os = "macos")]
const INBOX_IDENTIFIER_PREFIX: &str = "wtm-automation-inbox:";
#[cfg(target_os = "macos")]
const TARGET_KEY: &str = "target";

#[cfg(target_os = "macos")]
fn task_identifier(task_id: &str) -> String {
    format!("{IDENTIFIER_PREFIX}{task_id}")
}

/// An Inbox entry's notification never replaces, nor is removed with, its task's alerts.
#[cfg(target_os = "macos")]
fn notification_identifier(target: &NotificationTarget) -> String {
    match &target.inbox_entry {
        Some(entry) => format!("{INBOX_IDENTIFIER_PREFIX}{entry}"),
        None => task_identifier(&target.task_id),
    }
}

#[cfg(target_os = "macos")]
fn notifies_task(identifier: &str, task_ids: &[String]) -> bool {
    identifier
        .strip_prefix(IDENTIFIER_PREFIX)
        .is_some_and(|task_id| task_ids.iter().any(|id| id == task_id))
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
mod macos {
    use std::ffi::CStr;
    use std::path::Path;
    use std::process::Command;
    use std::sync::Mutex;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::AnyClass;
    use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{define_class, msg_send, AllocAnyThread, ClassType, DefinedClass};
    use objc2_foundation::{
        ns_string, NSDictionary, NSError, NSString, NSUserNotification, NSUserNotificationCenter,
        NSUserNotificationCenterDelegate,
    };
    use tauri::{AppHandle, Runtime};
    use tokio::sync::oneshot;

    use crate::commands::editor::GUI_LAUNCHER;

    #[cfg(not(test))]
    const AUTHORIZATION_CENTER: &CStr = c"UNUserNotificationCenter";
    #[cfg(test)]
    const AUTHORIZATION_CENTER: &CStr = super::tests::FAKE_AUTHORIZATION_CENTER;
    const NOTIFICATION_SETTINGS_URL: &str =
        "x-apple.systempreferences:com.apple.preference.notifications";

    #[link(name = "UserNotifications", kind = "framework")]
    extern "C" {}

    use super::{notification_identifier, notifies_task, NotificationTarget, TARGET_KEY};
    use crate::commands::agent_alerts::reveal_target;

    type OpenTarget = Box<dyn Fn(NotificationTarget)>;

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "WTMAgentNotificationDelegate"]
        #[ivars = OpenTarget]
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
                    self.ivars()(target);
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

    /// Unbundled dev builds borrow the app's bundle identifier, without which there is no
    /// notification center.
    fn center<R: Runtime>(app: &AppHandle<R>) -> Option<Retained<NSUserNotificationCenter>> {
        #[cfg(test)]
        if let Some(center) = super::tests::fake_center() {
            return center;
        }
        let _ = mac_notification_sys::set_application(&app.config().identifier);
        unsafe {
            msg_send![
                NSUserNotificationCenter::class(),
                defaultUserNotificationCenter
            ]
        }
    }

    /// The center does not retain its delegate, so it is kept alive for the app's lifetime.
    fn ensure_delegate<R: Runtime>(app: &AppHandle<R>, center: &NSUserNotificationCenter) {
        if unsafe { center.delegate() }.is_some() {
            return;
        }
        let app = app.clone();
        let open: OpenTarget = Box::new(move |target| reveal_target(&app, &target));
        let delegate = Delegate::alloc().set_ivars(open);
        let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
        unsafe { center.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
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

    fn notification(
        title: &str,
        body: &str,
        target: &NotificationTarget,
    ) -> Retained<NSUserNotification> {
        let notification = NSUserNotification::new();
        let identifier = NSString::from_str(&notification_identifier(target));
        notification.setIdentifier(Some(&identifier));
        notification.setTitle(Some(&NSString::from_str(title)));
        notification.setInformativeText(Some(&NSString::from_str(body)));
        let target_json = NSString::from_str(&serde_json::to_string(target).unwrap_or_default());
        let user_info = NSDictionary::<NSString, AnyObject>::from_slices(
            &[&*NSString::from_str(TARGET_KEY)],
            &[&**target_json],
        );
        unsafe { notification.setUserInfo(Some(&user_info)) };
        notification
    }

    fn on_center<R: Runtime>(
        app: &AppHandle<R>,
        action: impl FnOnce(&NSUserNotificationCenter) + Send + 'static,
    ) {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Some(center) = center(&handle) {
                ensure_delegate(&handle, &center);
                action(&center);
            }
        });
    }

    pub fn deliver<R: Runtime>(
        app: &AppHandle<R>,
        title: String,
        body: String,
        target: NotificationTarget,
    ) {
        on_center(app, move |center| {
            if target.inbox_entry.is_none() {
                remove_for_tasks(center, std::slice::from_ref(&target.task_id));
            }
            center.deliverNotification(&notification(&title, &body, &target));
        });
    }

    pub fn dismiss<R: Runtime>(app: &AppHandle<R>, task_ids: Vec<String>) {
        on_center(app, move |center| remove_for_tasks(center, &task_ids));
    }

    /// UserNotifications requires an actual application bundle. The identifier borrowed by the
    /// legacy center for development does not make the dev executable a bundle.
    pub async fn test_notification<R: Runtime>(
        app: &AppHandle<R>,
        executable: &Path,
    ) -> Result<bool, String> {
        if !executable
            .ancestors()
            .any(|path| path.extension() == Some("app".as_ref()))
        {
            return Err("Open the WorktreeManager .app to test macOS notifications.".into());
        }
        if !request_permission(app).await? {
            return Ok(false);
        }
        preview(app).await?;
        Ok(true)
    }

    async fn request_permission<R: Runtime>(app: &AppHandle<R>) -> Result<bool, String> {
        let (send, receive) = oneshot::channel();
        app.run_on_main_thread(move || {
            let send = Mutex::new(Some(send));
            let completion = RcBlock::new(move |granted: Bool, error: *mut NSError| {
                let result = match unsafe { error.as_ref() } {
                    Some(error) if is_not_allowed(error) => Ok(false),
                    Some(error) => Err(error.localizedDescription().to_string()),
                    None => Ok(granted.as_bool()),
                };
                if let Some(send) = send.lock().unwrap().take() {
                    let _ = send.send(result);
                }
            });
            let class = AnyClass::get(AUTHORIZATION_CENTER).expect("UserNotifications is linked");
            let center: Retained<AnyObject> = unsafe { msg_send![class, currentNotificationCenter] };
            const ALERT: usize = 1 << 2;
            let _: () = unsafe {
                msg_send![&*center, requestAuthorizationWithOptions: ALERT, completionHandler: &*completion]
            };
        })
        .map_err(|error| error.to_string())?;
        receive.await.map_err(|error| error.to_string())?
    }

    /// Unsigned or ad hoc signed builds report a denial as `UNErrorCodeNotificationsNotAllowed`.
    fn is_not_allowed(error: &NSError) -> bool {
        &*error.domain() == ns_string!("UNErrorDomain") && error.code() == 1
    }

    pub fn open_settings() -> Result<(), String> {
        Command::new(GUI_LAUNCHER)
            .arg(NOTIFICATION_SETTINGS_URL)
            .spawn()
            .map(drop)
            .map_err(|error| error.to_string())
    }

    pub async fn preview<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
        let (send, receive) = oneshot::channel();
        let handle = app.clone();
        app.run_on_main_thread(move || {
            let result = center(&handle)
                .ok_or_else(|| "macOS Notification Center is unavailable.".to_string())
                .map(|center| {
                    ensure_delegate(&handle, &center);
                    let notification = NSUserNotification::new();
                    notification.setIdentifier(Some(&NSString::from_str("wtm-agent-test")));
                    notification.setTitle(Some(&NSString::from_str("WorktreeManager")));
                    notification.setInformativeText(Some(&NSString::from_str(
                        "This is a test of your agent alerts.",
                    )));
                    center.deliverNotification(&notification);
                });
            let _ = send.send(result);
        })
        .map_err(|error| error.to_string())?;
        receive.await.map_err(|error| error.to_string())?
    }
}

#[tauri::command]
pub async fn agent_test_notification<R: Runtime>(app: AppHandle<R>) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        macos::test_notification(&app, &executable).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("Agent notifications are available on macOS.".into())
    }
}

#[tauri::command]
pub fn agent_open_notification_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        macos::open_settings()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Agent notifications are available on macOS.".into())
    }
}

#[cfg(target_os = "macos")]
pub use macos::{deliver, dismiss};

#[cfg(not(target_os = "macos"))]
pub fn deliver<R: Runtime>(
    _app: &AppHandle<R>,
    _title: String,
    _body: String,
    _target: NotificationTarget,
) {
}

#[cfg(not(target_os = "macos"))]
pub fn dismiss<R: Runtime>(_app: &AppHandle<R>, _task_ids: Vec<String>) {}

#[cfg(all(test, target_os = "macos"))]
#[allow(deprecated)]
pub(crate) mod tests {
    use std::cell::RefCell;
    use std::sync::{Arc, Mutex};

    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject};
    use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass, Message};
    use std::ffi::CStr;
    use std::path::Path;

    use block2::Block;
    use objc2::runtime::Bool;
    use objc2::ClassType;
    use objc2_foundation::{
        NSArray, NSError, NSString, NSUserNotification, NSUserNotificationCenter,
    };
    use serde_json::{json, Value};
    use tauri::Listener;

    use super::*;
    use crate::commands::agent_alerts::NOTIFICATION_OPEN_EVENT;

    #[derive(Default)]
    struct FakeCenterState {
        delivered: RefCell<Vec<Retained<NSUserNotification>>>,
        delegate: RefCell<Option<Retained<AnyObject>>>,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "WTMFakeNotificationCenter"]
        #[ivars = FakeCenterState]
        struct FakeCenter;

        impl FakeCenter {
            #[unsafe(method_id(deliveredNotifications))]
            fn delivered_notifications(&self) -> Retained<NSArray<NSUserNotification>> {
                NSArray::from_retained_slice(&self.ivars().delivered.borrow())
            }

            #[unsafe(method(deliverNotification:))]
            fn deliver_notification(&self, notification: &NSUserNotification) {
                self.ivars().delivered.borrow_mut().push(notification.retain());
            }

            #[unsafe(method(removeDeliveredNotification:))]
            fn remove_delivered_notification(&self, notification: &NSUserNotification) {
                self.ivars()
                    .delivered
                    .borrow_mut()
                    .retain(|delivered| !std::ptr::eq(&**delivered, notification));
            }

            #[unsafe(method_id(delegate))]
            fn delegate(&self) -> Option<Retained<AnyObject>> {
                self.ivars().delegate.borrow().clone()
            }

            #[unsafe(method(setDelegate:))]
            fn set_delegate(&self, delegate: Option<&AnyObject>) {
                *self.ivars().delegate.borrow_mut() = delegate.map(AnyObject::retain);
            }
        }
    );

    thread_local! {
        static FAKE_CENTER: RefCell<Option<Option<Retained<NSUserNotificationCenter>>>> =
            const { RefCell::new(None) };
    }

    /// `Some(None)` stands for a system without a notification center.
    pub(crate) fn fake_center() -> Option<Option<Retained<NSUserNotificationCenter>>> {
        FAKE_CENTER.with(|center| center.borrow().clone())
    }

    /// Routes this thread's notifications to a recording center instead of the system one.
    pub(crate) fn use_fake_center() -> Retained<NSUserNotificationCenter> {
        let fake = FakeCenter::alloc().set_ivars(FakeCenterState::default());
        let fake: Retained<FakeCenter> = unsafe { msg_send![super(fake), init] };
        let center = unsafe { Retained::cast_unchecked::<NSUserNotificationCenter>(fake) };
        FAKE_CENTER.with(|current| *current.borrow_mut() = Some(Some(center.clone())));
        center
    }

    pub(crate) fn delivered_identifiers(center: &NSUserNotificationCenter) -> Vec<String> {
        center
            .deliveredNotifications()
            .iter()
            .filter_map(|notification| notification.identifier())
            .map(|identifier| identifier.to_string())
            .collect()
    }

    pub(crate) const FAKE_AUTHORIZATION_CENTER: &CStr = c"WTMFakeAuthorizationCenter";

    thread_local! {
        static AUTHORIZATION: RefCell<(bool, Option<Retained<NSError>>)> =
            const { RefCell::new((false, None)) };
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "WTMFakeAuthorizationCenter"]
        struct FakeAuthorizationCenter;

        impl FakeAuthorizationCenter {
            #[unsafe(method_id(currentNotificationCenter))]
            fn current() -> Retained<Self> {
                unsafe { msg_send![Self::class(), new] }
            }

            #[unsafe(method(requestAuthorizationWithOptions:completionHandler:))]
            fn request_authorization(
                &self,
                _options: usize,
                completion: &Block<dyn Fn(Bool, *mut NSError)>,
            ) {
                AUTHORIZATION.with(|answer| {
                    let (granted, error) = &*answer.borrow();
                    let error = error.as_ref().map_or(std::ptr::null_mut(), |error| {
                        Retained::as_ptr(error).cast_mut()
                    });
                    completion.call((Bool::new(*granted), error));
                });
            }
        }
    );

    fn answer_authorization(granted: bool, error: Option<Retained<NSError>>) {
        FakeAuthorizationCenter::class();
        AUTHORIZATION.with(|answer| *answer.borrow_mut() = (granted, error));
    }

    fn error(domain: &str, code: isize) -> Retained<NSError> {
        unsafe { NSError::errorWithDomain_code_userInfo(&NSString::from_str(domain), code, None) }
    }

    fn test_from_bundle(app: &tauri::App<tauri::test::MockRuntime>) -> Result<bool, String> {
        let executable = Path::new("/Applications/WorktreeManager.app/Contents/MacOS/app");
        tauri::async_runtime::block_on(macos::test_notification(app.handle(), executable))
    }

    pub(crate) fn mock_app() -> tauri::App<tauri::test::MockRuntime> {
        use tauri::test::{mock_builder, mock_context, noop_assets};
        mock_builder().build(mock_context(noop_assets())).unwrap()
    }

    fn target(task_id: &str) -> NotificationTarget {
        serde_json::from_value(json!({ "taskId": task_id, "surface": "chat", "agent": "codex" }))
            .unwrap()
    }

    #[test]
    fn identifiers_match_only_their_own_tasks() {
        let ids = vec!["task-1".to_string(), "task-2".to_string()];
        assert!(notifies_task(&task_identifier("task-1"), &ids));
        assert!(notifies_task(&task_identifier("task-2"), &ids));
        assert!(!notifies_task(&task_identifier("task-10"), &ids));
        assert!(!notifies_task("task-1", &ids));
        assert!(!notifies_task("6F9619FF-8B86-D011-B42D-00C04FC964FF", &ids));
    }

    #[test]
    fn each_task_keeps_one_notification_until_dismissed() {
        let center = use_fake_center();
        let app = mock_app();
        deliver(app.handle(), "Done".into(), "WOR-1".into(), target("t1"));
        deliver(app.handle(), "Waiting".into(), "WOR-1".into(), target("t1"));
        deliver(app.handle(), "Done".into(), "WOR-2".into(), target("t2"));
        assert_eq!(
            delivered_identifiers(&center),
            [task_identifier("t1"), task_identifier("t2")]
        );
        let titles: Vec<String> = center
            .deliveredNotifications()
            .iter()
            .filter_map(|notification| notification.title())
            .map(|title| title.to_string())
            .collect();
        assert_eq!(titles, ["Waiting", "Done"]);

        dismiss(app.handle(), vec!["t1".into(), "unknown".into()]);
        assert_eq!(delivered_identifiers(&center), [task_identifier("t2")]);
    }

    #[test]
    fn test_notification_has_no_task_and_does_not_replace_an_alert() {
        let center = use_fake_center();
        let app = mock_app();
        deliver(app.handle(), "Done".into(), "WOR-1".into(), target("t1"));
        tauri::async_runtime::block_on(macos::preview(app.handle())).unwrap();
        assert_eq!(
            delivered_identifiers(&center),
            [task_identifier("t1"), "wtm-agent-test".into()]
        );
        assert!(center
            .deliveredNotifications()
            .objectAtIndex(1)
            .userInfo()
            .is_none());
    }

    fn inbox_target(task_id: &str, entry: &str) -> NotificationTarget {
        let mut target = target(task_id);
        target.inbox_entry = Some(entry.into());
        target
    }

    #[test]
    fn each_inbox_entry_keeps_its_own_notification_beside_its_tasks() {
        let center = use_fake_center();
        let app = mock_app();
        deliver(app.handle(), "Done".into(), "WOR-1".into(), target("t1"));
        for (task_id, entry) in [
            ("", "approval:a"),
            ("", "permission:b"),
            ("t1", "approval:c"),
        ] {
            deliver(
                app.handle(),
                "demo".into(),
                entry.into(),
                inbox_target(task_id, entry),
            );
        }
        assert_eq!(
            delivered_identifiers(&center),
            [
                task_identifier("t1"),
                "wtm-automation-inbox:approval:a".into(),
                "wtm-automation-inbox:permission:b".into(),
                "wtm-automation-inbox:approval:c".into(),
            ]
        );
    }

    #[test]
    fn clicking_a_notification_opens_its_target_and_removes_it() {
        let center = use_fake_center();
        let app = mock_app();
        let opened = Arc::new(Mutex::new(Vec::new()));
        let seen = opened.clone();
        app.listen_any(NOTIFICATION_OPEN_EVENT, move |event| {
            seen.lock().unwrap().push(event.payload().to_string());
        });
        let mut inbox_target = target("t1");
        inbox_target.inbox_entry = Some("run:demo:r1".into());
        deliver(app.handle(), "Done".into(), "WOR-1".into(), inbox_target);
        let delegate: Retained<AnyObject> = unsafe { msg_send![&*center, delegate] };
        let delivered = center.deliveredNotifications().objectAtIndex(0);
        let foreign = NSUserNotification::new();

        let present: bool = unsafe {
            msg_send![&*delegate, userNotificationCenter: &*center, shouldPresentNotification: &*delivered]
        };
        for notification in [&delivered, &foreign] {
            let _: () = unsafe {
                msg_send![&*delegate, userNotificationCenter: &*center, didActivateNotification: &**notification]
            };
        }

        assert!(present);
        assert!(delivered_identifiers(&center).is_empty());
        let opened = opened.lock().unwrap();
        assert_eq!(opened.len(), 1);
        assert_eq!(
            serde_json::from_str::<Value>(&opened[0]).unwrap(),
            json!({
                "taskId": "t1",
                "surface": "chat",
                "agent": "codex",
                "inboxEntry": "run:demo:r1",
            })
        );
    }

    #[test]
    fn without_a_notification_center_nothing_is_delivered() {
        FAKE_CENTER.with(|current| *current.borrow_mut() = Some(None));
        let app = mock_app();
        deliver(app.handle(), "Done".into(), "WOR-1".into(), target("t1"));
        dismiss(app.handle(), vec!["t1".into()]);
        let result = tauri::async_runtime::block_on(macos::preview(app.handle()));
        assert_eq!(
            result.unwrap_err(),
            "macOS Notification Center is unavailable."
        );
    }

    #[test]
    fn testing_outside_an_app_bundle_asks_to_open_the_app() {
        let app = mock_app();
        let result = tauri::async_runtime::block_on(agent_test_notification(app.handle().clone()));
        assert_eq!(
            result.unwrap_err(),
            "Open the WorktreeManager .app to test macOS notifications."
        );
    }

    #[test]
    fn a_granted_permission_sends_the_test_notification() {
        let center = use_fake_center();
        let app = mock_app();
        answer_authorization(true, None);
        assert_eq!(test_from_bundle(&app), Ok(true));
        assert_eq!(delivered_identifiers(&center), ["wtm-agent-test"]);
    }

    #[test]
    fn a_denied_permission_sends_nothing() {
        let center = use_fake_center();
        let app = mock_app();
        answer_authorization(false, None);
        assert_eq!(test_from_bundle(&app), Ok(false));
        assert!(delivered_identifiers(&center).is_empty());
    }

    #[test]
    fn notifications_not_allowed_count_as_a_denial() {
        let center = use_fake_center();
        let app = mock_app();
        answer_authorization(false, Some(error("UNErrorDomain", 1)));
        assert_eq!(test_from_bundle(&app), Ok(false));
        assert!(delivered_identifiers(&center).is_empty());
    }

    #[test]
    fn other_authorization_errors_are_reported() {
        use_fake_center();
        let app = mock_app();
        answer_authorization(false, Some(error("WTMTestDomain", 7)));
        assert!(test_from_bundle(&app)
            .unwrap_err()
            .contains("WTMTestDomain"));
    }

    #[test]
    fn notification_settings_open() {
        assert_eq!(agent_open_notification_settings(), Ok(()));
    }
}
