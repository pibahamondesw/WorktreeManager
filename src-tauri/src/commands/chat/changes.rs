//! File changes between turn checkpoints. Checkpoints cover every task repository and edits made
//! by any tool, so the same review works for both agents.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use tauri::State;

use super::checkpoints::{self, Checkpoints, FileComparison};
use super::{blocking, model, ChatRegistry};

const DIFF_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

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
    let cache = format!(
        "changes-{}.json",
        checkpoints::hash(format!("{before}\0{after}").as_bytes())
    );
    if let Ok(set) = checkpoints.read::<ChangeSet>(&cache) {
        return Ok(set);
    }
    let comparisons = checkpoints.compare(before, after)?;
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
    ChatChanges {
        turns: turns
            .into_iter()
            .filter_map(|(items, turn)| {
                logged(change_set(checkpoints, &turn.before, &turn.after))
                    .map(|changes| TurnChanges { items, changes })
            })
            .collect(),
        conversation,
    }
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

fn image_mime(path: &str) -> Option<&'static str> {
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    IMAGE_TYPES
        .iter()
        .find(|(known, _)| *known == extension)
        .map(|(_, mime)| *mime)
}

fn content(path: &str, bytes: Vec<u8>) -> FileContent {
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
    let read = |id: &str, path: &str| {
        checkpoints
            .stored_file(id, folder, path)?
            .map(|file| {
                checkpoints
                    .read_file(&file)
                    .map(|bytes| content(path, bytes))
            })
            .transpose()
    };
    Ok(FileDiff {
        before: read(before, previous_path)?,
        after: read(after, path)?,
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
    agent: &str,
) -> Result<SessionState, String> {
    registry.with_session(task_id, agent, |session| {
        let state = session.shared.lock().unwrap();
        Ok(SessionState {
            checkpoints: state.checkpoints.clone(),
            conversation: state.conversation_id.clone(),
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

#[tauri::command]
pub async fn chat_changes(
    registry: State<'_, ChatRegistry>,
    task_id: String,
    agent: String,
) -> Result<ChatChanges, String> {
    let state = session_state(&registry, &task_id, &agent)?;
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
) -> Result<FileDiff, String> {
    let checkpoints = session_state(&registry, &task_id, &agent)?
        .checkpoints
        .ok_or("Checkpoints are unavailable")?;
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
