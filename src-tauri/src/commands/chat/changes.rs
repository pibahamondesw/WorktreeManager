//! File changes between turn checkpoints. Checkpoints cover every task repository and edits made
//! by any tool, so the same review works for both agents.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use tauri::State;

use super::checkpoints::{self, Checkpoints, FileComparison};
use super::{blocking, model, ChatRegistry};
use crate::commands::git::git_command;

const DIFF_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
static REVIEWED_WRITES: Mutex<()> = Mutex::new(());
const GENERATED_ATTRIBUTES: [&str; 2] = ["linguist-generated", "gitlab-generated"];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub folder: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_path: Option<String>,
    pub status: ChangeStatus,
    pub additions: usize,
    pub deletions: usize,
    pub binary: bool,
    /// Identifies the file's resulting content: the after blob, or the removed blob for
    /// deletions. A later turn that changes the file changes it too.
    pub content: String,
    #[serde(default)]
    pub generated: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSet {
    pub before: String,
    pub after: String,
    pub files: Vec<FileChange>,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnChanges {
    pub items: Vec<String>,
    pub changes: ChangeSet,
    pub branch: Option<ChangeSet>,
}

#[derive(Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatChanges {
    pub turns: Vec<TurnChanges>,
    pub conversation: Option<ChangeSet>,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FileContent {
    Text { text: String },
    Image { mime: String, data: String },
    Binary { size: usize },
}

#[derive(Debug, PartialEq, Serialize)]
pub struct FileDiff {
    pub before: Option<FileContent>,
    pub after: Option<FileContent>,
}

fn as_text(bytes: &[u8]) -> Option<&str> {
    if bytes.len() > MAX_TEXT_BYTES || bytes.iter().take(8000).any(|byte| *byte == 0) {
        return None;
    }
    std::str::from_utf8(bytes).ok()
}

fn line_counts(old: &str, new: &str) -> (usize, usize) {
    TextDiff::configure()
        .timeout(DIFF_TIMEOUT)
        .diff_lines(old, new)
        .iter_all_changes()
        .fold((0, 0), |(additions, deletions), change| {
            match change.tag() {
                ChangeTag::Insert => (additions + 1, deletions),
                ChangeTag::Delete => (additions, deletions + 1),
                ChangeTag::Equal => (additions, deletions),
            }
        })
}

fn file_change(
    checkpoints: &Checkpoints,
    comparison: &FileComparison,
) -> Result<FileChange, String> {
    let read = |file: &Option<checkpoints::StoredFile>| {
        file.as_ref()
            .map(|file| checkpoints.read_file(file))
            .transpose()
    };
    let before = read(&comparison.before)?;
    let after = read(&comparison.after)?;
    let texts = (
        before.as_deref().map(as_text),
        after.as_deref().map(as_text),
    );
    let binary = matches!(texts, (Some(None), _) | (_, Some(None)));
    let (additions, deletions) = match texts {
        (Some(Some(old)), Some(Some(new))) => line_counts(old, new),
        (None, Some(Some(new))) => line_counts("", new),
        (Some(Some(old)), None) => line_counts(old, ""),
        _ => (0, 0),
    };
    Ok(FileChange {
        folder: comparison.folder.clone(),
        path: comparison.path.clone(),
        previous_path: None,
        status: match (&comparison.before, &comparison.after) {
            (None, _) => ChangeStatus::Added,
            (_, None) => ChangeStatus::Deleted,
            _ => ChangeStatus::Modified,
        },
        additions,
        deletions,
        binary,
        content: comparison
            .after
            .as_ref()
            .map(|after| after.blob().to_string())
            .or_else(|| {
                let before = comparison.before.as_ref()?;
                Some(format!("deleted:{}", before.blob()))
            })
            .unwrap_or_default(),
        generated: false,
    })
}

fn with_renames(comparisons: &[FileComparison], files: Vec<FileChange>) -> Vec<FileChange> {
    let mut deleted = BTreeMap::new();
    for (index, comparison) in comparisons.iter().enumerate() {
        if let (Some(before), None) = (&comparison.before, &comparison.after) {
            deleted
                .entry((comparison.folder.as_str(), before.blob()))
                .or_insert(index);
        }
    }
    let mut sources = vec![None; files.len()];
    for (index, comparison) in comparisons.iter().enumerate() {
        if let (None, Some(after)) = (&comparison.before, &comparison.after) {
            sources[index] = deleted.remove(&(comparison.folder.as_str(), after.blob()));
        }
    }
    let renamed = sources.iter().flatten().collect::<BTreeSet<_>>();
    files
        .iter()
        .enumerate()
        .filter(|(index, _)| !renamed.contains(index))
        .map(|(index, file)| match sources[index] {
            Some(source) => FileChange {
                status: ChangeStatus::Renamed,
                previous_path: Some(files[source].path.clone()),
                additions: 0,
                deletions: 0,
                ..file.clone()
            },
            None => file.clone(),
        })
        .collect()
}

pub fn change_set(
    checkpoints: &Checkpoints,
    before: &str,
    after: &str,
) -> Result<ChangeSet, String> {
    let baseline = checkpoints.baseline(before, after)?;
    let cache = format!(
        "changes-v2-{}.json",
        checkpoints::hash(format!("{before}\0{after}{}", baseline.key()).as_bytes())
    );
    if let Ok(set) = checkpoints.read::<ChangeSet>(&cache) {
        return Ok(set);
    }
    let comparisons = checkpoints.compare(&baseline);
    let files = comparisons
        .iter()
        .map(|comparison| file_change(checkpoints, comparison))
        .collect::<Result<Vec<_>, _>>()?;
    let files = with_renames(&comparisons, files);
    let set = ChangeSet {
        before: before.to_string(),
        after: after.to_string(),
        files,
    };
    let _ = checkpoints.save(&cache, &set);
    Ok(set)
}

fn branch_change_set(checkpoints: &Checkpoints, point: &str) -> Result<ChangeSet, String> {
    change_set(checkpoints, &checkpoints.upstream_checkpoint(point)?, point)
}

fn generated_paths(folder: &str, paths: &BTreeSet<String>) -> Result<BTreeSet<String>, String> {
    let mut child = git_command()
        .arg("-C")
        .arg(folder)
        .args(["check-attr", "--stdin", "-z"])
        .args(GENERATED_ATTRIBUTES)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let input = paths
        .iter()
        .map(|path| format!("{path}\0"))
        .collect::<String>();
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let _ = writer.join();
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let fields = stdout.split('\0').collect::<Vec<_>>();
    Ok(fields
        .chunks_exact(3)
        .filter(|entry| matches!(entry[2], "set" | "true"))
        .map(|entry| entry[0].to_string())
        .collect())
}

fn change_sets_mut(changes: &mut ChatChanges) -> impl Iterator<Item = &mut ChangeSet> {
    changes.conversation.iter_mut().chain(
        changes
            .turns
            .iter_mut()
            .flat_map(|turn| std::iter::once(&mut turn.changes).chain(turn.branch.iter_mut())),
    )
}

/// Marks files that `.gitattributes` declares generated. Attributes come from the current
/// worktree, so a failure to read them only leaves files expanded.
fn mark_generated(changes: &mut ChatChanges) {
    let mut paths_by_folder = BTreeMap::<String, BTreeSet<String>>::new();
    for set in change_sets_mut(changes) {
        for file in &set.files {
            paths_by_folder
                .entry(file.folder.clone())
                .or_default()
                .insert(file.path.clone());
        }
    }
    let generated = paths_by_folder
        .into_iter()
        .map(|(folder, paths)| {
            let found = generated_paths(&folder, &paths).unwrap_or_else(|error| {
                log::warn!("could not read generated attributes in {folder}: {error}");
                BTreeSet::new()
            });
            (folder, found)
        })
        .collect::<BTreeMap<_, _>>();
    for set in change_sets_mut(changes) {
        for file in &mut set.files {
            file.generated = generated
                .get(&file.folder)
                .is_some_and(|paths| paths.contains(&file.path));
        }
    }
}

fn logged(set: Result<ChangeSet, String>) -> Option<ChangeSet> {
    set.map_err(|error| log::warn!("could not compute chat changes: {error}"))
        .ok()
}

pub fn collect(
    checkpoints: &Checkpoints,
    conversation: &str,
    user_items: &[String],
) -> ChatChanges {
    let mut turns: Vec<(Vec<String>, checkpoints::TurnCheckpoints)> = Vec::new();
    for item in user_items {
        let Some(turn) = checkpoints.find_turn(conversation, item) else {
            continue;
        };
        match turns.last_mut() {
            Some((items, last)) if *last == turn => items.push(item.clone()),
            _ => turns.push((vec![item.clone()], turn)),
        }
    }
    let conversation = match (turns.first(), turns.last()) {
        (Some((_, first)), Some((_, last))) => {
            logged(change_set(checkpoints, &first.before, &last.after))
        }
        _ => None,
    };
    let mut changes = ChatChanges {
        turns: turns
            .into_iter()
            .filter_map(|(items, turn)| {
                logged(change_set(checkpoints, &turn.before, &turn.after)).map(|changes| {
                    TurnChanges {
                        items,
                        branch: logged(branch_change_set(checkpoints, &turn.after)),
                        changes,
                    }
                })
            })
            .collect(),
        conversation,
    };
    mark_generated(&mut changes);
    changes
}

const IMAGE_TYPES: [(&str, &str); 8] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("bmp", "image/bmp"),
    ("ico", "image/x-icon"),
    ("avif", "image/avif"),
];

pub(super) fn image_mime(path: &str) -> Option<&'static str> {
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    IMAGE_TYPES
        .iter()
        .find(|(known, _)| *known == extension)
        .map(|(_, mime)| *mime)
}

pub(super) fn content(path: &str, bytes: Vec<u8>) -> FileContent {
    if let Some(mime) = image_mime(path).filter(|_| bytes.len() <= MAX_IMAGE_BYTES) {
        return FileContent::Image {
            mime: mime.into(),
            data: base64::engine::general_purpose::STANDARD.encode(&bytes),
        };
    }
    match as_text(&bytes) {
        Some(text) => FileContent::Text { text: text.into() },
        None => FileContent::Binary { size: bytes.len() },
    }
}

pub fn file_diff(
    checkpoints: &Checkpoints,
    before: &str,
    after: &str,
    folder: &str,
    path: &str,
    previous_path: Option<&str>,
) -> Result<FileDiff, String> {
    let previous_path = previous_path.unwrap_or(path);
    let baseline = checkpoints.baseline(before, after)?;
    let read = |file: Option<checkpoints::StoredFile>, path: &str| {
        file.map(|file| {
            checkpoints
                .read_file(&file)
                .map(|bytes| content(path, bytes))
        })
        .transpose()
    };
    Ok(FileDiff {
        before: read(
            checkpoints.before_file(&baseline, folder, previous_path),
            previous_path,
        )?,
        after: read(checkpoints.after_file(&baseline, folder, path), path)?,
    })
}

struct SessionState {
    checkpoints: Option<Checkpoints>,
    conversation: Option<String>,
    user_items: Vec<String>,
}

fn session_state(
    registry: &ChatRegistry,
    task_id: &str,
    slot: &str,
) -> Result<SessionState, String> {
    registry.with_session(task_id, slot, |session| {
        let state = session.shared.lock().unwrap();
        Ok(SessionState {
            checkpoints: state.turns.checkpoints().cloned(),
            conversation: state.turns.conversation().map(String::from),
            user_items: state
                .snapshot
                .items
                .iter()
                .filter(|item| item.kind == model::ItemKind::User)
                .map(|item| item.id.clone())
                .collect(),
        })
    })
}

fn reviewed_file_name(conversation: &str) -> String {
    format!(
        "reviewed-{}.json",
        checkpoints::hash(conversation.as_bytes())
    )
}

pub fn reviewed_files(checkpoints: &Checkpoints, conversation: &str) -> BTreeSet<String> {
    checkpoints
        .read(&reviewed_file_name(conversation))
        .unwrap_or_default()
}

pub fn set_file_reviewed(
    checkpoints: &Checkpoints,
    conversation: &str,
    key: &str,
    reviewed: bool,
) -> Result<BTreeSet<String>, String> {
    let _guard = REVIEWED_WRITES.lock().unwrap_or_else(|e| e.into_inner());
    let mut keys = reviewed_files(checkpoints, conversation);
    if reviewed {
        keys.insert(key.to_string());
    } else {
        keys.remove(key);
    }
    checkpoints.save(&reviewed_file_name(conversation), &keys)?;
    Ok(keys)
}

struct ConversationStore {
    checkpoints: Checkpoints,
    conversation: String,
}

fn conversation_store(
    registry: &ChatRegistry,
    task_id: &str,
    agent: &str,
    tab: Option<&str>,
) -> Result<ConversationStore, String> {
    let state = session_state(registry, task_id, &super::tabs::slot(agent, tab))?;
    match (state.checkpoints, state.conversation) {
        (Some(checkpoints), Some(conversation)) => Ok(ConversationStore {
            checkpoints,
            conversation,
        }),
        _ => Err("This conversation has no checkpoints yet".into()),
    }
}

#[tauri::command]
pub async fn chat_reviewed_files(
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
    tab: Option<String>,
) -> Result<BTreeSet<String>, String> {
    let store = conversation_store(&registry, &task_id, &agent, tab.as_deref())?;
    blocking(move || Ok(reviewed_files(&store.checkpoints, &store.conversation))).await
}

#[tauri::command]
pub async fn chat_set_file_reviewed(
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
    tab: Option<String>,
    key: String,
    reviewed: bool,
) -> Result<BTreeSet<String>, String> {
    let store = conversation_store(&registry, &task_id, &agent, tab.as_deref())?;
    blocking(move || set_file_reviewed(&store.checkpoints, &store.conversation, &key, reviewed))
        .await
}

#[tauri::command]
pub async fn chat_changes(
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
    tab: Option<String>,
) -> Result<ChatChanges, String> {
    let state = session_state(
        &registry,
        &task_id,
        &super::tabs::slot(&agent, tab.as_deref()),
    )?;
    let (Some(checkpoints), Some(conversation)) = (state.checkpoints, state.conversation) else {
        return Ok(ChatChanges::default());
    };
    blocking(move || Ok(collect(&checkpoints, &conversation, &state.user_items))).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn chat_file_diff(
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
    before: String,
    after: String,
    folder: String,
    path: String,
    previous_path: Option<String>,
    tab: Option<String>,
) -> Result<FileDiff, String> {
    let slot = super::tabs::slot(&agent, tab.as_deref());
    let state = session_state(&registry, &task_id, &slot)?;
    let checkpoints = state.checkpoints.ok_or("Checkpoints are unavailable")?;
    blocking(move || {
        file_diff(
            &checkpoints,
            &before,
            &after,
            &folder,
            &path,
            previous_path.as_deref(),
        )
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::chat::checkpoints::tests::Fixture;
    use std::fs;

    fn capture(fixture: &Fixture) -> String {
        fixture.store.capture(&fixture.folders).unwrap()
    }

    #[test]
    fn summarizes_turns_and_the_whole_conversation_across_repositories() {
        let fixture = Fixture::new(2);
        let checkpoints = &fixture.store;
        let first = capture(&fixture);
        fs::write(fixture.file(0, "file.txt"), "one\ntwo\nthree\n").unwrap();
        let second = capture(&fixture);
        fs::write(fixture.file(1, "new.txt"), "added\n").unwrap();
        fs::remove_file(fixture.file(0, "file.txt")).unwrap();
        let third = capture(&fixture);
        let turns = [
            ("turn-1", &first, &second),
            ("queued", &first, &second),
            ("turn-2", &second, &third),
            ("no-turn", &third, &third),
        ];
        for (item, before, after) in &turns[..3] {
            let turn = checkpoints::TurnCheckpoints {
                before: before.to_string(),
                after: after.to_string(),
            };
            checkpoints.bind_turn("conv", item, &turn).unwrap();
        }
        let items = turns.map(|(item, _, _)| item.to_string());

        let changes = collect(checkpoints, "conv", &items);

        assert_eq!(changes.turns.len(), 2);
        assert_eq!(changes.turns[0].items, ["turn-1", "queued"]);
        let first_file = &changes.turns[0].changes.files;
        assert_eq!(first_file.len(), 1);
        assert_eq!(first_file[0].folder, fixture.folders[0]);
        assert_eq!(first_file[0].path, "file.txt");
        assert_eq!(first_file[0].status, ChangeStatus::Modified);
        assert_eq!((first_file[0].additions, first_file[0].deletions), (3, 1));
        let second_files = &changes.turns[1].changes.files;
        assert_eq!(
            second_files
                .iter()
                .map(|f| (f.path.as_str(), f.status, f.additions, f.deletions))
                .collect::<Vec<_>>(),
            [
                ("file.txt", ChangeStatus::Deleted, 0, 3),
                ("new.txt", ChangeStatus::Added, 1, 0),
            ]
        );
        let conversation = changes.conversation.unwrap();
        assert_eq!(conversation.before, first);
        assert_eq!(conversation.after, third);
        assert_eq!(conversation.files[0].status, ChangeStatus::Deleted);
        assert_eq!(conversation.files[1].folder, fixture.folders[1]);
    }

    #[test]
    fn content_ids_follow_the_resulting_file_across_turns() {
        let fixture = Fixture::new(1);
        let checkpoints = &fixture.store;
        let first = capture(&fixture);
        fs::write(fixture.file(0, "kept.txt"), "kept\n").unwrap();
        fs::write(fixture.file(0, "file.txt"), "edited\n").unwrap();
        let second = capture(&fixture);
        fs::write(fixture.file(0, "file.txt"), "edited again\n").unwrap();
        let third = capture(&fixture);
        fs::remove_file(fixture.file(0, "kept.txt")).unwrap();
        let fourth = capture(&fixture);
        let content = |before: &str, after: &str, path: &str| {
            change_set(checkpoints, before, after)
                .unwrap()
                .files
                .into_iter()
                .find(|file| file.path == path)
                .unwrap()
                .content
        };

        assert_eq!(
            content(&first, &second, "kept.txt"),
            content(&first, &third, "kept.txt")
        );
        assert_ne!(
            content(&first, &second, "file.txt"),
            content(&first, &third, "file.txt")
        );
        assert_eq!(
            content(&third, &fourth, "kept.txt"),
            format!("deleted:{}", content(&first, &second, "kept.txt"))
        );
    }

    #[test]
    fn reviewed_files_persist_per_conversation() {
        let fixture = Fixture::new(1);
        let checkpoints = &fixture.store;

        set_file_reviewed(checkpoints, "conv", "a", true).unwrap();
        set_file_reviewed(checkpoints, "conv", "b", true).unwrap();
        set_file_reviewed(checkpoints, "conv", "a", false).unwrap();

        let reloaded = Checkpoints::new(&fixture.dir.join("app-data"));
        assert_eq!(
            reviewed_files(&reloaded, "conv")
                .into_iter()
                .collect::<Vec<_>>(),
            ["b"]
        );
        assert!(reviewed_files(&reloaded, "other").is_empty());
    }

    #[test]
    fn marks_files_that_gitattributes_declares_generated_in_every_change_set() {
        let fixture = Fixture::new(1);
        let checkpoints = &fixture.store;
        fs::write(
            fixture.file(0, ".gitattributes"),
            "*.gen.ts linguist-generated\nschema.json gitlab-generated=true\nkept.gen.ts -linguist-generated\n",
        )
        .unwrap();
        let before = capture(&fixture);
        for name in ["api.gen.ts", "schema.json", "kept.gen.ts", "app.ts"] {
            fs::write(fixture.file(0, name), "content\n").unwrap();
        }
        let after = capture(&fixture);
        let turn = checkpoints::TurnCheckpoints {
            before: before.clone(),
            after,
        };
        checkpoints.bind_turn("conv", "turn-1", &turn).unwrap();

        let changes = collect(checkpoints, "conv", &["turn-1".to_string()]);

        let generated = |set: &ChangeSet| {
            set.files
                .iter()
                .filter(|file| file.generated)
                .map(|file| file.path.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            generated(changes.conversation.as_ref().unwrap()),
            ["api.gen.ts", "schema.json"]
        );
        assert_eq!(
            generated(&changes.turns[0].changes),
            ["api.gen.ts", "schema.json"]
        );
    }

    #[test]
    fn leaves_files_expanded_when_attributes_cannot_be_read() {
        let folder = std::env::temp_dir().join(format!("wtm-no-repo-{}", uuid::Uuid::new_v4()));
        let file = FileChange {
            folder: folder.to_string_lossy().to_string(),
            path: "api.gen.ts".into(),
            previous_path: None,
            status: ChangeStatus::Added,
            additions: 1,
            deletions: 0,
            binary: false,
            content: "blob".into(),
            generated: true,
        };
        let mut changes = ChatChanges {
            turns: Vec::new(),
            conversation: Some(ChangeSet {
                before: "b".into(),
                after: "a".into(),
                files: vec![file],
            }),
        };

        mark_generated(&mut changes);

        assert!(!changes.conversation.unwrap().files[0].generated);
    }

    #[test]
    fn detects_exact_renames_and_reads_both_sides() {
        let fixture = Fixture::new(1);
        let checkpoints = &fixture.store;
        fs::write(fixture.file(0, "old.rs"), "fn main() {}\n").unwrap();
        fs::write(fixture.file(0, "notes.md"), "same\nold\n").unwrap();
        let before = capture(&fixture);
        fs::rename(fixture.file(0, "old.rs"), fixture.file(0, "new.rs")).unwrap();
        fs::write(fixture.file(0, "notes.md"), "same\nnew\n").unwrap();
        let after = capture(&fixture);

        let set = change_set(checkpoints, &before, &after).unwrap();

        assert_eq!(set.files.len(), 2);
        assert_eq!(set.files[0].status, ChangeStatus::Renamed);
        assert_eq!(set.files[0].previous_path.as_deref(), Some("old.rs"));
        assert_eq!((set.files[0].additions, set.files[0].deletions), (0, 0));
        assert_eq!(set.files[1].path, "notes.md");
        assert_eq!((set.files[1].additions, set.files[1].deletions), (1, 1));
        let diff = file_diff(
            checkpoints,
            &before,
            &after,
            &fixture.folders[0],
            "new.rs",
            Some("old.rs"),
        )
        .unwrap();
        let text = Some(FileContent::Text {
            text: "fn main() {}\n".into(),
        });
        assert_eq!(diff.before, text);
        assert_eq!(diff.after, text);
        assert_eq!(change_set(checkpoints, &before, &after).unwrap(), set);
    }

    #[test]
    fn returns_images_and_binary_files_without_line_counts() {
        let fixture = Fixture::new(1);
        let checkpoints = &fixture.store;
        let before = capture(&fixture);
        fs::write(fixture.file(0, "logo.png"), [0x89, b'P', b'N', b'G', 0]).unwrap();
        fs::write(fixture.file(0, "data.bin"), [1, 0, 2]).unwrap();
        let after = capture(&fixture);

        let set = change_set(checkpoints, &before, &after).unwrap();

        assert!(set
            .files
            .iter()
            .all(|file| file.binary && file.additions == 0 && file.deletions == 0));
        let image = file_diff(
            checkpoints,
            &before,
            &after,
            &fixture.folders[0],
            "logo.png",
            None,
        )
        .unwrap();
        assert_eq!(image.before, None);
        assert_eq!(
            image.after,
            Some(FileContent::Image {
                mime: "image/png".into(),
                data: "iVBORwA=".into(),
            })
        );
        let binary = file_diff(
            checkpoints,
            &before,
            &after,
            &fixture.folders[0],
            "data.bin",
            None,
        )
        .unwrap();
        assert_eq!(binary.after, Some(FileContent::Binary { size: 3 }));
    }

    fn run(fixture: &Fixture, args: &[&str]) {
        let status = crate::commands::git::git_command()
            .args(["-C", &fixture.folders[0]])
            .args(args)
            .output()
            .unwrap();
        assert!(status.status.success(), "git {args:?}: {status:?}");
    }

    fn commit(fixture: &Fixture, name: &str, text: &str) {
        fs::write(fixture.file(0, name), text).unwrap();
        run(fixture, &["add", name]);
        run(fixture, &["commit", "-qm", name]);
    }

    fn feature_branch(shared: &str) -> Fixture {
        let fixture = Fixture::new(1);
        run(&fixture, &["branch", "-M", "main"]);
        commit(&fixture, "shared.txt", shared);
        run(&fixture, &["checkout", "-qb", "feature"]);
        fixture
    }

    fn advance_main_and_rebase(fixture: &Fixture, shared: &str) {
        run(fixture, &["checkout", "-q", "main"]);
        commit(fixture, "upstream.txt", "from main\n");
        commit(fixture, "shared.txt", shared);
        run(fixture, &["checkout", "-q", "feature"]);
        run(fixture, &["rebase", "-q", "main"]);
    }

    fn text(text: &str) -> Option<FileContent> {
        Some(FileContent::Text { text: text.into() })
    }

    #[test]
    fn upstream_changes_brought_by_a_rebase_are_not_reported() {
        let fixture = feature_branch("a\nb\nc\n");
        commit(&fixture, "shared.txt", "a\nb\nc\nagent\n");
        let before = capture(&fixture);
        advance_main_and_rebase(&fixture, "upstream\na\nb\nc\n");
        fs::write(
            fixture.file(0, "shared.txt"),
            "upstream\na\nb\nc\nagent\nturn\n",
        )
        .unwrap();
        let after = capture(&fixture);

        let set = change_set(&fixture.store, &before, &after).unwrap();

        assert_eq!(
            set.files
                .iter()
                .map(|f| (f.path.as_str(), f.status, f.additions, f.deletions))
                .collect::<Vec<_>>(),
            [("shared.txt", ChangeStatus::Modified, 1, 0)]
        );
        let diff = file_diff(
            &fixture.store,
            &before,
            &after,
            &fixture.folders[0],
            "shared.txt",
            None,
        )
        .unwrap();
        assert_eq!(diff.before, text("upstream\na\nb\nc\nagent\n"));
        assert_eq!(diff.after, text("upstream\na\nb\nc\nagent\nturn\n"));
        let untouched = file_diff(
            &fixture.store,
            &before,
            &after,
            &fixture.folders[0],
            "file.txt",
            None,
        )
        .unwrap();
        assert_eq!(untouched.before, untouched.after);
    }

    #[test]
    fn each_turn_reports_the_whole_branch_diff_up_to_its_end() {
        let fixture = feature_branch("a\n");
        commit(&fixture, "feature.txt", "committed\n");
        let before = capture(&fixture);
        advance_main_and_rebase(&fixture, "upstream\na\n");
        fs::write(fixture.file(0, "shared.txt"), "upstream\na\nturn\n").unwrap();
        let after = capture(&fixture);
        let turn = checkpoints::TurnCheckpoints { before, after };
        fixture.store.bind_turn("conv", "user", &turn).unwrap();

        let changes = collect(&fixture.store, "conv", &["user".into()]);

        let branch = changes.turns[0].branch.as_ref().unwrap();
        assert_eq!(branch.after, turn.after);
        assert_eq!(
            branch
                .files
                .iter()
                .map(|f| (f.path.as_str(), f.status, f.additions, f.deletions))
                .collect::<Vec<_>>(),
            [
                ("feature.txt", ChangeStatus::Added, 1, 0),
                ("shared.txt", ChangeStatus::Modified, 1, 0),
            ]
        );
        let diff = file_diff(
            &fixture.store,
            &branch.before,
            &branch.after,
            &fixture.folders[0],
            "shared.txt",
            None,
        )
        .unwrap();
        assert_eq!(diff.before, text("upstream\na\n"));
    }

    #[test]
    fn edits_conflicting_with_upstream_keep_the_original_before() {
        let fixture = feature_branch("a\nb\nc\n");
        fs::write(fixture.file(0, "shared.txt"), "a\nmine\nc\n").unwrap();
        fs::remove_file(fixture.file(0, "file.txt")).unwrap();
        let before = capture(&fixture);
        run(&fixture, &["checkout", "--", "shared.txt", "file.txt"]);
        run(&fixture, &["checkout", "-q", "main"]);
        commit(&fixture, "file.txt", "theirs\n");
        run(&fixture, &["checkout", "-q", "feature"]);
        advance_main_and_rebase(&fixture, "a\ntheirs\nc\n");
        let after = capture(&fixture);

        let set = change_set(&fixture.store, &before, &after).unwrap();

        assert_eq!(
            set.files
                .iter()
                .map(|f| (f.path.as_str(), f.additions, f.deletions))
                .collect::<Vec<_>>(),
            [("file.txt", 1, 0), ("shared.txt", 1, 1)]
        );
    }

    #[test]
    fn commits_on_the_feature_or_upstream_branch_itself_are_reported() {
        let fixture = feature_branch("a\n");
        let before = capture(&fixture);
        commit(&fixture, "shared.txt", "a\nfeature\n");
        let after = capture(&fixture);
        assert_eq!(
            change_set(&fixture.store, &before, &after).unwrap().files[0].path,
            "shared.txt"
        );

        run(&fixture, &["checkout", "-q", "main"]);
        let before = capture(&fixture);
        commit(&fixture, "shared.txt", "a\nmain\n");
        let after = capture(&fixture);
        assert_eq!(
            change_set(&fixture.store, &before, &after).unwrap().files[0].path,
            "shared.txt"
        );
    }

    #[test]
    fn agent_commits_stay_reported_after_the_branch_is_merged_upstream() {
        let fixture = feature_branch("a\n");
        let before = capture(&fixture);
        commit(&fixture, "shared.txt", "a\nagent\n");
        let after = capture(&fixture);
        run(&fixture, &["checkout", "-q", "main"]);
        run(&fixture, &["merge", "-q", "--ff-only", "feature"]);
        run(&fixture, &["checkout", "-q", "feature"]);

        let set = change_set(&fixture.store, &before, &after).unwrap();

        assert_eq!(set.files.len(), 1);
        assert_eq!(set.files[0].path, "shared.txt");
    }

    #[test]
    fn upstream_symlink_retargets_are_not_reported() {
        let fixture = feature_branch("a\n");
        let retarget_on_main = |target: &str| {
            run(&fixture, &["checkout", "-q", "main"]);
            let _ = fs::remove_file(fixture.file(0, "link"));
            std::os::unix::fs::symlink(target, fixture.file(0, "link")).unwrap();
            run(&fixture, &["add", "link"]);
            run(&fixture, &["commit", "-qm", target]);
            run(&fixture, &["checkout", "-q", "feature"]);
            run(&fixture, &["rebase", "-q", "main"]);
        };
        retarget_on_main("shared.txt");
        let before = capture(&fixture);
        retarget_on_main("file.txt");
        let after = capture(&fixture);

        assert_eq!(
            change_set(&fixture.store, &before, &after).unwrap().files,
            []
        );
    }

    #[test]
    fn upstream_mode_changes_are_not_reported() {
        let fixture = feature_branch("a\n");
        let before = capture(&fixture);
        run(&fixture, &["checkout", "-q", "main"]);
        fs::set_permissions(
            fixture.file(0, "shared.txt"),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        run(&fixture, &["add", "shared.txt"]);
        run(&fixture, &["commit", "-qm", "executable"]);
        run(&fixture, &["checkout", "-q", "feature"]);
        run(&fixture, &["rebase", "-q", "main"]);
        let after = capture(&fixture);

        assert_eq!(
            change_set(&fixture.store, &before, &after).unwrap().files,
            []
        );
    }

    #[test]
    fn damaged_checkpoint_data_is_reported() {
        let fixture = Fixture::new(1);
        let checkpoints = &fixture.store;
        let before = capture(&fixture);
        fs::write(fixture.file(0, "new.txt"), "content\n").unwrap();
        let after = capture(&fixture);
        let blobs = fixture.dir.join("app-data/chat-checkpoints/blobs");
        for blob in fs::read_dir(&blobs).unwrap() {
            fs::write(blob.unwrap().path(), "damaged").unwrap();
        }

        assert!(change_set(checkpoints, &before, &after)
            .unwrap_err()
            .contains("damaged"));
        assert_eq!(collect(checkpoints, "conv", &[]), ChatChanges::default());
        let turn = checkpoints::TurnCheckpoints { before, after };
        checkpoints.bind_turn("conv", "user", &turn).unwrap();
        assert_eq!(
            collect(checkpoints, "conv", &["user".into()]),
            ChatChanges::default()
        );
    }
}
