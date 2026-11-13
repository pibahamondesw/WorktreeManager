use tauri::Manager;

#[cfg(unix)]
use std::sync::Arc;

use commands::automations::actions::circleci;

#[cfg(unix)]
pub mod cli;
#[cfg(unix)]
mod cli_bridge;
mod commands;
#[cfg(unix)]
mod lease;
mod lifecycle;
mod menu;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .manage(commands::terminal::TerminalRegistry::default())
        .manage(commands::chat::ChatRegistry::default())
        .manage(commands::code_server::EditorRegistry::default())
        .manage(lifecycle::Background::default())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            std::thread::spawn(commands::chat::purge_stale_chat_images);
            app.manage(circleci::CircleciToken(std::sync::Arc::new(KeychainItem {
                service: app.config().identifier.clone(),
                account: CIRCLECI_TOKEN_ACCOUNT,
            })));
            #[cfg(unix)]
            app.manage(
                cli_bridge::CliBridge::start(cli_bridge::socket_path(&app.config().identifier))
                    .map_err(std::io::Error::other)?,
            );
            #[cfg(unix)]
            if let Err(error) = start_automations(app.handle()) {
                log::warn!("Automations are off for this session: {error}");
            }
            menu::setup(app)?;
            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;
            Ok(())
        })
        .invoke_handler({
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
                menu::set_shortcut_menu,
                menu::set_shortcut_capture,
                menu::set_shortcut_context,
                cli_bridge::cli_bridge_register,
                cli_bridge::cli_bridge_unregister,
                cli_bridge::cli_bridge_progress,
                cli_bridge::cli_bridge_complete,
                commands::validation::validate_workspace_repos,
                commands::validation::validate_task_worktrees,
                commands::git::git_worktree_add,
                commands::git::resolve_manual_worktree,
                commands::git::git_user_slug,
                commands::git::copy_local_configs,
                commands::git::git_worktree_remove,
                commands::git::git_worktree_list,
                commands::git::git_worktree_status,
                commands::git::git_worktree_status_batch,
                commands::git::git_remote_url,
                commands::doctor::doctor_probe,
                commands::github::github_pr_status_batch,
                commands::github::github_pr_ready,
                commands::vault_agents::sync_vault_agent,
                commands::vault_agents::probe_vault_agent,
                commands::worktree_cleanup::cleanup_removed_worktrees,
                commands::detected_setup::prepare_detected_setup,
                commands::detected_setup::run_detected_setup,
                commands::editor::open_editor,
                commands::editor::check_app_installed,
                commands::export::save_png,
                commands::workspace::remove_task_workspace,
                commands::notes::ensure_task_note,
                commands::notes::archive_task_note,
                commands::notes::append_task_note,
                commands::vault::scaffold_vault,
                commands::vault::ensure_vault,
                commands::claude_config::cleanup_claude_json_stale,
                commands::agents::agents_catalog,
                commands::automations::definition::automations_list,
                commands::automations::definition::automations_get,
                commands::automations::store::automation_runs_list,
                commands::automations::store::automation_run_get,
                commands::automations::store::automation_state,
                commands::automations::actions::automation_run,
                commands::automations::actions::automation_pause,
                commands::automations::actions::automation_resume,
                commands::automations::store::automation_items_list,
                commands::automations::operator::automation_next,
                commands::automations::operator::automation_continuous,
                commands::automations::operator::automation_set_limit,
                commands::automations::decisions::automation_decide,
                commands::automations::decisions::automation_revoke_decision,
                commands::automations::actions::fire::automation_fire,
                commands::automations::actions::events::automation_app_event,
                commands::automations::actions::automation_signal,
                commands::automations::actions::intervene::automation_take_over,
                commands::automations::actions::intervene::automation_hand_back,
                commands::automations::actions::intervene::automation_retry,
                commands::automations::command::view::automation_step_receipt,
                commands::agent_alerts::agent_hooks_status,
                commands::agent_alerts::agent_hooks_install,
                commands::agent_alerts::agent_hooks_remove,
                commands::agent_alerts::agent_attention,
                commands::agent_alerts::agent_badge,
                commands::agent_alerts::agent_dismiss_notifications,
                commands::keychain::keychain_get,
                commands::keychain::keychain_set,
                circleci::token::circleci_token_saved,
                circleci::token::circleci_token_save,
                commands::terminal::terminal_open,
                commands::terminal::terminal_write,
                commands::terminal::terminal_resize,
                commands::terminal::terminal_detach,
                commands::terminal::terminal_close,
                commands::terminal::terminal_list,
                commands::terminal::terminal_stop,
                commands::terminal::terminal_attach,
                commands::terminal::run_task_script,
                commands::terminal::script_cancel,
                commands::scripts::resolve_repo_scripts,
                commands::scripts::suggest_repo_setup,
                commands::scripts::write_repo_setup,
                commands::chat::chat_stop,
                commands::chat::chat_open,
                commands::chat::chat_send,
                commands::chat::history::chat_rewind_preview,
                commands::chat::history::chat_history,
                commands::chat::changes::chat_changes,
                commands::chat::changes::chat_file_diff,
                commands::chat::chat_interrupt,
                commands::chat::chat_steer_queued,
                commands::chat::chat_unqueue,
                commands::chat::chat_configure,
                commands::chat::chat_compact,
                commands::chat::chat_review,
                commands::chat::chat_refresh_usage,
                commands::agent_usage::agent_cached_usage,
                commands::chat::chat_file_search,
                commands::chat::chat_read_image,
                commands::chat::chat_read_file,
                commands::chat::chat_read_markdown,
                commands::chat::chat_respond,
                commands::chat::chat_detach,
                commands::chat::chat_close,
                commands::chat::chat_list,
                commands::chat::chat_sessions,
                commands::chat::chat_session_rename,
                commands::chat::chat_session_archive,
                commands::chat::interrupted::chat_interrupted_list,
                commands::chat::interrupted::chat_interrupted_clear,
                lifecycle::set_surface_turn_in_progress,
                commands::code_server::vscode_probe,
                commands::code_server::vscode_install,
                commands::code_server::vscode_install_status,
                commands::code_server::vscode_open,
                commands::embedded::vscode_present,
                commands::code_server::vscode_close,
                commands::code_server::vscode_list,
                commands::opencode::opencode_open,
                commands::opencode::opencode_close,
                commands::opencode::opencode_list,
            ];
            move |invoke| {
                if invoke.message.webview_ref().label() != "main" {
                    invoke
                        .resolver
                        .reject("WTM commands are available only to its main interface");
                    true
                } else {
                    handler(invoke)
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(lifecycle::on_run_event);
}

/// Runs started by the scheduler and by the owner share one executor.
#[cfg(unix)]
fn start_automations(app: &tauri::AppHandle) -> Result<(), String> {
    use commands::automations::{actions, routine, scheduler};
    let operations = Arc::new(cli_bridge::AppOperations(app.clone()));
    let leases = app.state::<cli_bridge::CliBridge>().leases();
    let executor: Arc<dyn scheduler::Executor> =
        Arc::new(routine::RoutineExecutor::of(app, operations)?.with_lease_pools(Arc::new(leases)));
    app.manage(actions::AutomationEngine(executor.clone()));
    start_linear_triggers(app, executor.clone())?;
    start_github_triggers(app, executor.clone())?;
    start_circleci_triggers(app, executor.clone())?;
    scheduler::start_scheduler(app.clone(), executor)
}

/// Polls GitHub once a minute for the automations with GitHub event triggers.
#[cfg(unix)]
fn start_github_triggers(
    app: &tauri::AppHandle,
    executor: Arc<dyn commands::automations::scheduler::Executor>,
) -> Result<(), String> {
    use commands::automations::{actions::github, definition, store::RunStore};
    let store = RunStore::of(app)?;
    let mut triggers = github::GithubTriggers::new(Arc::new(github::CliGithub::default()));
    let app = app.clone();
    std::thread::spawn(move || {
        log::info!("GitHub triggers poll every minute");
        loop {
            match definition::load_current(&app) {
                Ok(entries) => {
                    let definitions: Vec<_> = entries
                        .iter()
                        .filter_map(|e| e.definition.as_ref())
                        .collect();
                    let now = std::time::SystemTime::now().into();
                    triggers.poll(&store, executor.as_ref(), &definitions, now);
                }
                Err(error) => log::warn!("GitHub triggers could not load the automations: {error}"),
            }
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    });
    Ok(())
}

/// Polls Linear once a minute for the automations with Linear event triggers.
#[cfg(unix)]
fn start_linear_triggers(
    app: &tauri::AppHandle,
    executor: Arc<dyn commands::automations::scheduler::Executor>,
) -> Result<(), String> {
    use commands::automations::{actions::linear, definition, store::RunStore};
    let store = RunStore::of(app)?;
    let keychain = app.clone();
    let secrets: linear::SecretReader =
        Arc::new(move || commands::keychain::keychain_get(keychain.clone()));
    let mut triggers = linear::LinearTriggers::new(Arc::new(linear::GraphqlLinear::new(secrets)));
    let app = app.clone();
    std::thread::spawn(move || {
        log::info!("Linear triggers poll every minute");
        loop {
            match definition::load_current(&app) {
                Ok(entries) => {
                    let definitions: Vec<_> = entries
                        .iter()
                        .filter_map(|e| e.definition.as_ref())
                        .collect();
                    let now = std::time::SystemTime::now().into();
                    triggers.poll(&store, executor.as_ref(), &definitions, now);
                }
                Err(error) => log::warn!("Linear triggers could not load the automations: {error}"),
            }
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    });
    Ok(())
}

const CIRCLECI_TOKEN_ACCOUNT: &str = "circleci-token";

/// A Keychain item of the app's own service, apart from the Linear keys the frontend owns.
struct KeychainItem {
    service: String,
    account: &'static str,
}

impl circleci::SecretItem for KeychainItem {
    fn read(&self) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(&self.service, self.account)
            .map_err(|e| format!("Failed to open keychain entry: {e}"))?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("Failed to read from keychain: {e}")),
        }
    }

    fn write(&self, value: Option<&str>) -> Result<(), String> {
        let entry = keyring::Entry::new(&self.service, self.account)
            .map_err(|e| format!("Failed to open keychain entry: {e}"))?;
        let written = match value {
            Some(value) => entry.set_password(value),
            None => match entry.delete_credential() {
                Err(keyring::Error::NoEntry) => Ok(()),
                deleted => deleted,
            },
        };
        written.map_err(|e| format!("Failed to write to keychain: {e}"))
    }
}

/// Polls CircleCI once a minute for the automations with CircleCI event triggers.
#[cfg(unix)]
fn start_circleci_triggers(
    app: &tauri::AppHandle,
    executor: Arc<dyn commands::automations::scheduler::Executor>,
) -> Result<(), String> {
    use commands::automations::{definition, store::RunStore};
    let store = RunStore::of(app)?;
    let saved = app.state::<circleci::CircleciToken>().inner().clone();
    let token: circleci::TokenReader = Arc::new(move || saved.read());
    let mut triggers =
        circleci::CircleciTriggers::new(Arc::new(circleci::RestCircleci::new(token)));
    let app = app.clone();
    std::thread::spawn(move || {
        log::info!("CircleCI triggers poll every minute");
        loop {
            match definition::load_current(&app) {
                Ok(entries) => {
                    let definitions: Vec<_> = entries
                        .iter()
                        .filter_map(|e| e.definition.as_ref())
                        .collect();
                    let now = std::time::SystemTime::now().into();
                    triggers.poll(&store, executor.as_ref(), &definitions, now);
                }
                Err(error) => {
                    log::warn!("CircleCI triggers could not load the automations: {error}")
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    });
    Ok(())
}
