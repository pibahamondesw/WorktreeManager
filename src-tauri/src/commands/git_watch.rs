use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter, Runtime, State};

pub const GIT_STATUS_CHANGED_EVENT: &str = "git-status-changed";
const POLL_INTERVAL: Duration = Duration::from_secs(1);
const STATUS_FILES: [&str; 3] = ["HEAD", "index", "logs/HEAD"];

type Signature = [Option<SystemTime>; 3];

#[derive(Default)]
pub struct GitWatchRegistry(Mutex<Option<Sender<Vec<String>>>>);

#[tauri::command]
pub fn git_status_watch<R: Runtime>(
    app: AppHandle<R>,
    registry: State<'_, GitWatchRegistry>,
    worktree_paths: Vec<String>,
) -> Result<(), String> {
    let mut worker = registry.0.lock().map_err(|e| e.to_string())?;
    worker
        .get_or_insert_with(|| spawn_poll_worker(app))
        .send(worktree_paths)
        .map_err(|e| e.to_string())
}

fn spawn_poll_worker<R: Runtime>(app: AppHandle<R>) -> Sender<Vec<String>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        run_poll_loop(receiver, POLL_INTERVAL, |paths| {
            let _ = app.emit(GIT_STATUS_CHANGED_EVENT, paths);
        })
    });
    sender
}

fn run_poll_loop(
    receiver: Receiver<Vec<String>>,
    interval: Duration,
    mut emit: impl FnMut(Vec<String>),
) {
    let mut poller = StatusPoller::default();
    loop {
        let received = if poller.is_idle() {
            receiver.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            receiver.recv_timeout(interval)
        };
        match received {
            Ok(worktree_paths) => poller.replace(&worktree_paths),
            Err(RecvTimeoutError::Timeout) => {
                let settled = poller.poll();
                if !settled.is_empty() {
                    emit(settled);
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[derive(Default)]
struct StatusPoller {
    watched: HashMap<String, (PathBuf, Signature)>,
    pending: HashSet<String>,
}

impl StatusPoller {
    fn is_idle(&self) -> bool {
        self.watched.is_empty()
    }

    fn replace(&mut self, worktree_paths: &[String]) {
        let mut previous = std::mem::take(&mut self.watched);
        for worktree in worktree_paths {
            let entry = previous.remove(worktree).or_else(|| {
                let git_dir = git_dir(Path::new(worktree))?;
                let signature = status_signature(&git_dir);
                Some((git_dir, signature))
            });
            if let Some(entry) = entry {
                self.watched.insert(worktree.clone(), entry);
            }
        }
        self.pending
            .retain(|worktree| self.watched.contains_key(worktree));
    }

    fn poll(&mut self) -> Vec<String> {
        let mut changing = HashSet::new();
        for (worktree, (git_dir, signature)) in &mut self.watched {
            let current = status_signature(git_dir);
            if current != *signature {
                *signature = current;
                changing.insert(worktree.clone());
            }
        }
        let mut settled: Vec<String> = self.pending.difference(&changing).cloned().collect();
        settled.sort();
        self.pending.retain(|worktree| changing.contains(worktree));
        self.pending.extend(changing);
        settled
    }
}

fn git_dir(worktree: &Path) -> Option<PathBuf> {
    let dot_git = worktree.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let content = std::fs::read_to_string(&dot_git).ok()?;
    let target = content.trim().strip_prefix("gitdir:")?.trim();
    Some(worktree.join(target))
}

fn status_signature(git_dir: &Path) -> Signature {
    STATUS_FILES.map(|file| {
        std::fs::metadata(git_dir.join(file))
            .and_then(|metadata| metadata.modified())
            .ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("wtm-git-watch-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_worktree(root: &Path, name: &str) -> (String, PathBuf) {
        let worktree = root.join(name);
        let git_dir = root.join("repo/.git/worktrees").join(name);
        std::fs::create_dir_all(git_dir.join("logs")).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", git_dir.display()),
        )
        .unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        (worktree.to_string_lossy().to_string(), git_dir)
    }

    fn touch(path: &Path, seconds: u64) {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
            .unwrap();
    }

    #[test]
    fn git_dir_resolves_main_checkout_and_linked_worktree() {
        let root = temp_dir("resolve");
        let main = root.join("repo");
        std::fs::create_dir_all(main.join(".git")).unwrap();
        let (linked, linked_git_dir) = fake_worktree(&root, "feature");

        assert_eq!(git_dir(&main), Some(main.join(".git")));
        assert_eq!(git_dir(Path::new(&linked)), Some(linked_git_dir));
        assert_eq!(git_dir(&root.join("missing")), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reports_a_worktree_once_its_status_files_stop_changing() {
        let root = temp_dir("poll");
        let (feature, feature_git_dir) = fake_worktree(&root, "feature");
        let (other, _) = fake_worktree(&root, "other");
        let mut poller = StatusPoller::default();
        poller.replace(&[feature.clone(), other]);
        assert!(poller.poll().is_empty());

        touch(&feature_git_dir.join("index"), 1);
        assert!(poller.poll().is_empty());
        touch(&feature_git_dir.join("HEAD"), 2);
        assert!(poller.poll().is_empty());
        touch(&feature_git_dir.join("logs/HEAD"), 3);
        assert!(poller.poll().is_empty());
        assert_eq!(poller.poll(), vec![feature]);
        assert!(poller.poll().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replacing_paths_keeps_known_signatures_and_drops_removed_worktrees() {
        let root = temp_dir("replace");
        let (feature, feature_git_dir) = fake_worktree(&root, "feature");
        let (other, other_git_dir) = fake_worktree(&root, "other");
        let mut poller = StatusPoller::default();
        poller.replace(std::slice::from_ref(&feature));
        touch(&feature_git_dir.join("index"), 1);
        poller.poll();
        poller.replace(&[feature.clone(), other]);
        assert_eq!(poller.poll(), vec![feature.clone()]);

        touch(&other_git_dir.join("index"), 1);
        poller.poll();
        poller.replace(std::slice::from_ref(&feature));
        assert!(poller.poll().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn command_emits_the_changed_worktree_to_the_frontend() {
        use tauri::test::{mock_builder, mock_context, noop_assets};
        use tauri::{Listener, Manager};

        let root = temp_dir("command");
        let (feature, feature_git_dir) = fake_worktree(&root, "feature");
        let app = mock_builder()
            .manage(GitWatchRegistry::default())
            .build(mock_context(noop_assets()))
            .unwrap();
        let (emitted_sender, emitted) = mpsc::channel();
        app.listen_any(GIT_STATUS_CHANGED_EVENT, move |event| {
            let _ = emitted_sender.send(event.payload().to_string());
        });

        git_status_watch(app.handle().clone(), app.state(), vec![feature.clone()]).unwrap();
        std::thread::sleep(POLL_INTERVAL / 2);
        touch(&feature_git_dir.join("HEAD"), 7);

        assert_eq!(
            emitted.recv_timeout(POLL_INTERVAL * 5).unwrap(),
            serde_json::to_string(&[feature]).unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn poll_loop_reports_changes_and_stops_when_disconnected() {
        let root = temp_dir("loop");
        let (feature, feature_git_dir) = fake_worktree(&root, "feature");
        let (sender, receiver) = mpsc::channel();
        let (emitted_sender, emitted) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            run_poll_loop(receiver, Duration::from_millis(20), |paths| {
                emitted_sender.send(paths).unwrap()
            })
        });
        sender.send(vec![feature.clone()]).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        touch(&feature_git_dir.join("HEAD"), 5);

        assert_eq!(
            emitted.recv_timeout(Duration::from_secs(2)).unwrap(),
            vec![feature]
        );
        drop(sender);
        worker.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
