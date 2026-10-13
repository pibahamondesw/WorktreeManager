use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::checkpoint_git::{self, BaseFile};
use crate::commands::git::git_command;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FileState {
    blob: String,
    mode: u32,
    link: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    git_blob: Option<String>,
}

impl PartialEq for FileState {
    fn eq(&self, other: &Self) -> bool {
        self.blob == other.blob && self.mode == other.mode && self.link == other.link
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnCheckpoints {
    pub before: String,
    pub after: String,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Confirmation {
    Current { checkpoint: String, backup: String },
    Legacy(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Repository {
    folder: String,
    head: String,
    branch: String,
    files: BTreeMap<String, FileState>,
    #[serde(default)]
    delta: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    removed: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    repositories: Vec<Repository>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewindPreview {
    pub token: String,
    pub files: Vec<String>,
}

pub struct Checkpoints {
    dir: PathBuf,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn git(folder: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = git_command()
        .arg("-C")
        .arg(folder)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(output.stdout)
}

fn safe_path(folder: &str, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
        || relative.is_empty()
    {
        return Err("Invalid checkpoint path".into());
    }
    let mut parent = PathBuf::from(folder);
    for part in path.parent().into_iter().flat_map(Path::components) {
        parent.push(part);
        if fs::symlink_metadata(&parent).is_ok_and(|m| !m.is_dir() || m.file_type().is_symlink()) {
            return Err(format!("Cannot restore through {}", parent.display()));
        }
    }
    Ok(Path::new(folder).join(path))
}

impl Checkpoints {
    pub fn new(app_data: &Path) -> Self {
        Self {
            dir: app_data.join("chat-checkpoints"),
        }
    }

    fn save<T: Serialize>(&self, name: &str, value: &T) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let path = self.dir.join(name);
        let tmp = self.dir.join(format!("{}.tmp", uuid::Uuid::new_v4()));
        fs::write(&tmp, serde_json::to_vec(value).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        fs::rename(tmp, path).map_err(|e| e.to_string())
    }

    fn read<T: serde::de::DeserializeOwned>(&self, name: &str) -> Result<T, String> {
        serde_json::from_slice(&fs::read(self.dir.join(name)).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }

    pub fn capture(&self, folders: &[String]) -> Result<String, String> {
        self.capture_known(folders, None)
    }

    pub fn capture_following(
        &self,
        folders: &[String],
        previous: Option<&str>,
    ) -> Result<String, String> {
        match previous {
            Some(id) => self.capture_known(folders, Some(&self.load(id)?)),
            None => self.capture(folders),
        }
    }

    fn capture_known(
        &self,
        folders: &[String],
        known: Option<&Checkpoint>,
    ) -> Result<String, String> {
        let checkpoint = self.compact(self.scan(folders, known, true)?)?;
        let bytes = serde_json::to_vec(&checkpoint).map_err(|e| e.to_string())?;
        let digest = Sha256::digest(&bytes);
        let id = uuid::Uuid::from_bytes(digest[..16].try_into().unwrap()).to_string();
        if !self.dir.join(format!("{id}.json")).exists() {
            self.save(&format!("{id}.json"), &checkpoint)?;
        }
        Ok(id)
    }

    fn base(&self, folder: &str, head: &str) -> Result<BTreeMap<String, BaseFile>, String> {
        let key = format!("base-{}.json", hash(format!("{folder}\0{head}").as_bytes()));
        if let Ok(base) = self.read(&key) {
            return Ok(base);
        }
        let base = checkpoint_git::read_base(folder, head.trim())?;
        self.save(&key, &base)?;
        Ok(base)
    }

    fn compact(&self, mut checkpoint: Checkpoint) -> Result<Checkpoint, String> {
        for repository in &mut checkpoint.repositories {
            let base = self.base(&repository.folder, &repository.head)?;
            repository.removed = base
                .keys()
                .filter(|path| !repository.files.contains_key(*path))
                .cloned()
                .collect();
            repository.files.retain(|path, file| match base.get(path) {
                Some(base) => *file != base_state(base),
                None => true,
            });
            repository.delta = true;
        }
        Ok(checkpoint)
    }

    fn scan(
        &self,
        folders: &[String],
        known: Option<&Checkpoint>,
        save_blobs: bool,
    ) -> Result<Checkpoint, String> {
        fs::create_dir_all(self.dir.join("blobs")).map_err(|e| e.to_string())?;
        let mut repositories = Vec::new();
        for folder in folders {
            let metadata = String::from_utf8(git(
                folder,
                &[
                    "rev-parse",
                    "--show-toplevel",
                    "HEAD",
                    "--abbrev-ref",
                    "HEAD",
                ],
            )?)
            .map_err(|e| e.to_string())?;
            let mut metadata = metadata.trim_end_matches('\n').rsplitn(3, '\n');
            let branch = format!("{}\n", metadata.next().ok_or("Git branch is missing")?);
            let head = format!("{}\n", metadata.next().ok_or("Git HEAD is missing")?);
            let root = metadata.next().ok_or("Git repository root is missing")?;
            if fs::canonicalize(root.trim()).ok() != fs::canonicalize(folder).ok() {
                return Err("Checkpoints require repository roots".into());
            }
            let base = self.base(folder, &head)?;
            let raw_paths = git(
                folder,
                &[
                    "ls-files",
                    "--cached",
                    "--others",
                    "--exclude-standard",
                    "-z",
                ],
            )?;
            let mut paths = base.keys().cloned().collect::<BTreeSet<_>>();
            for path in raw_paths
                .split(|byte| *byte == 0)
                .filter(|path| !path.is_empty())
            {
                paths.insert(
                    std::str::from_utf8(path)
                        .map_err(|_| "Non-UTF-8 file names cannot be checkpointed")?
                        .to_string(),
                );
            }
            if let Some(repository) = known
                .and_then(|checkpoint| checkpoint.repositories.iter().find(|r| &r.folder == folder))
            {
                paths.extend(repository.files.keys().cloned());
            }
            let mut files = BTreeMap::new();
            for relative in paths {
                let relative = relative.as_str();
                let path = safe_path(folder, relative)?;
                let metadata = match fs::symlink_metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e.to_string()),
                };
                let link = metadata.file_type().is_symlink();
                if !link && !metadata.is_file() {
                    return Err(format!(
                        "Cannot checkpoint {} (submodules are unsupported)",
                        path.display()
                    ));
                }
                let bytes = if link {
                    fs::read_link(&path)
                        .map_err(|e| e.to_string())?
                        .as_os_str()
                        .as_bytes()
                        .to_vec()
                } else {
                    fs::read(&path).map_err(|e| e.to_string())?
                };
                let blob = hash(&bytes);
                let git_blob = base
                    .get(relative)
                    .filter(|file| file.hash == blob)
                    .map(|file| file.object.clone());
                let destination = self.dir.join("blobs").join(&blob);
                if git_blob.is_none() && save_blobs && !destination.exists() {
                    let tmp = self
                        .dir
                        .join("blobs")
                        .join(format!("{}.tmp", uuid::Uuid::new_v4()));
                    fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
                    fs::rename(tmp, destination).map_err(|e| e.to_string())?;
                }
                files.insert(
                    relative.to_string(),
                    FileState {
                        blob,
                        mode: metadata.permissions().mode(),
                        link,
                        git_blob,
                    },
                );
            }
            repositories.push(Repository {
                folder: folder.clone(),
                head,
                branch,
                files,
                delta: false,
                removed: Vec::new(),
            });
        }
        Ok(Checkpoint { repositories })
    }

    pub fn load(&self, id: &str) -> Result<Checkpoint, String> {
        uuid::Uuid::parse_str(id).map_err(|_| "Invalid checkpoint")?;
        let mut checkpoint: Checkpoint = self.read(&format!("{id}.json"))?;
        for repository in &mut checkpoint.repositories {
            if repository.delta {
                let base = self.base(&repository.folder, &repository.head)?;
                let removed = repository.removed.iter().cloned().collect::<BTreeSet<_>>();
                for (path, base) in base {
                    if !removed.contains(&path) {
                        repository
                            .files
                            .entry(path)
                            .or_insert_with(|| base_state(&base));
                    }
                }
                repository.delta = false;
                repository.removed.clear();
            }
        }
        Ok(checkpoint)
    }

    pub fn ensure_folders(&self, id: &str, folders: &[String]) -> Result<(), String> {
        let checkpoint = self.load(id)?;
        if checkpoint
            .repositories
            .iter()
            .map(|repository| &repository.folder)
            .ne(folders.iter())
        {
            return Err("Task repositories changed since this checkpoint".into());
        }
        Ok(())
    }

    pub fn bind(&self, conversation: &str, item: &str, checkpoint: &str) -> Result<(), String> {
        self.save(
            &format!(
                "message-{}.json",
                hash(format!("{conversation}\0{item}").as_bytes())
            ),
            &checkpoint,
        )
    }

    pub fn find(&self, conversation: &str, item: &str) -> Option<String> {
        self.read(&format!(
            "message-{}.json",
            hash(format!("{conversation}\0{item}").as_bytes())
        ))
        .ok()
    }

    pub fn bind_turn(
        &self,
        conversation: &str,
        item: &str,
        turn: &TurnCheckpoints,
    ) -> Result<(), String> {
        self.save(
            &format!(
                "turn-{}.json",
                hash(format!("{conversation}\0{item}").as_bytes())
            ),
            turn,
        )
    }

    pub fn find_turn(&self, conversation: &str, item: &str) -> Option<TurnCheckpoints> {
        self.read(&format!(
            "turn-{}.json",
            hash(format!("{conversation}\0{item}").as_bytes())
        ))
        .ok()
    }

    pub fn preview(&self, id: &str) -> Result<RewindPreview, String> {
        let target = self.load(id)?;
        let folders = target
            .repositories
            .iter()
            .map(|r| r.folder.clone())
            .collect::<Vec<_>>();
        let backup = self.capture_known(&folders, Some(&target))?;
        let current = self.load(&backup)?;
        let files = changes(&target, &current)?;
        let token = uuid::Uuid::new_v4().to_string();
        self.save(
            &format!("preview-{token}.json"),
            &Confirmation::Current {
                checkpoint: id.to_string(),
                backup,
            },
        )?;
        Ok(RewindPreview { token, files })
    }

    pub fn validate(&self, id: &str, token: &str) -> Result<(), String> {
        self.approved_backup(id, token).map(|_| ())
    }

    fn approved_backup(&self, id: &str, token: &str) -> Result<String, String> {
        uuid::Uuid::parse_str(token).map_err(|_| "Invalid confirmation")?;
        let (approved, backup_id) =
            match self.read::<Confirmation>(&format!("preview-{token}.json"))? {
                Confirmation::Current { checkpoint, backup } => (checkpoint, backup),
                Confirmation::Legacy(checkpoint) => (checkpoint, token.to_string()),
            };
        if approved != id {
            return Err("Confirmation belongs to another checkpoint".into());
        }
        let target = self.load(id)?;
        let backup = self.load(&backup_id)?;
        let folders = target
            .repositories
            .iter()
            .map(|r| r.folder.clone())
            .collect::<Vec<_>>();
        let current = self.scan(&folders, Some(&backup), false)?;
        if current != backup {
            return Err("Files changed since the preview; review them again".into());
        }
        changes(&target, &current)?;
        Ok(backup_id)
    }

    pub fn restore(&self, id: &str, token: &str) -> Result<String, String> {
        let backup = self.approved_backup(id, token)?;
        let target = self.load(id)?;
        let current = self.load(&backup)?;
        let mut writes = Vec::new();
        for (target, current) in target.repositories.iter().zip(&current.repositories) {
            for name in target
                .files
                .keys()
                .chain(current.files.keys())
                .collect::<BTreeSet<_>>()
            {
                if target.files.get(name) == current.files.get(name) {
                    continue;
                }
                let path = safe_path(&target.folder, name)?;
                if fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
                    return Err(format!("Cannot overwrite directory {}", path.display()));
                }
                let file = target.files.get(name).cloned();
                let bytes = file
                    .as_ref()
                    .map(|f| {
                        if let Some(object) = &f.git_blob {
                            checkpoint_git::read_blob(&target.folder, object)
                        } else {
                            fs::read(self.dir.join("blobs").join(&f.blob))
                                .map_err(|e| e.to_string())
                        }
                    })
                    .transpose()?;
                if let (Some(file), Some(bytes)) = (&file, &bytes) {
                    if hash(bytes) != file.blob {
                        return Err("Checkpoint data is damaged; no files were restored".into());
                    }
                }
                writes.push((path, file, bytes));
            }
        }
        for (path, file, bytes) in writes {
            let result = (|| -> std::io::Result<()> {
                if let Some(file) = file {
                    fs::create_dir_all(path.parent().unwrap())?;
                    let tmp = path.with_file_name(format!(".wtm-{}", uuid::Uuid::new_v4()));
                    let bytes = bytes.unwrap();
                    if file.link {
                        symlink(std::ffi::OsStr::from_bytes(&bytes), &tmp)?;
                    } else {
                        fs::write(&tmp, bytes)?;
                        fs::set_permissions(&tmp, fs::Permissions::from_mode(file.mode))?;
                    }
                    fs::rename(tmp, path)?;
                } else if path.exists() || fs::symlink_metadata(&path).is_ok() {
                    fs::remove_file(path)?;
                }
                Ok(())
            })();
            if let Err(e) = result {
                return Err(format!(
                    "Restore partially failed: {e}. Recovery checkpoint: {backup}"
                ));
            }
        }
        Ok(backup)
    }
}

fn base_state(base: &BaseFile) -> FileState {
    FileState {
        blob: base.hash.clone(),
        mode: base.mode,
        link: base.mode & 0o170000 == 0o120000,
        git_blob: Some(base.object.clone()),
    }
}

fn changes(target: &Checkpoint, current: &Checkpoint) -> Result<Vec<String>, String> {
    if target.repositories.len() != current.repositories.len() {
        return Err("Task repositories changed".into());
    }
    let mut files = Vec::new();
    for (target, current) in target.repositories.iter().zip(&current.repositories) {
        if target.folder != current.folder
            || target.head != current.head
            || target.branch != current.branch
        {
            return Err("The branch or HEAD changed since this checkpoint".into());
        }
        for name in target
            .files
            .keys()
            .chain(current.files.keys())
            .collect::<BTreeSet<_>>()
        {
            if target.files.get(name) != current.files.get(name) {
                files.push(format!("{}/{}", target.folder, name));
            }
        }
    }
    Ok(files)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub(crate) struct Fixture {
        pub(crate) dir: PathBuf,
        pub(crate) folders: Vec<String>,
        pub(crate) store: Checkpoints,
    }

    impl Fixture {
        pub(crate) fn new(count: usize) -> Self {
            let dir =
                std::env::temp_dir().join(format!("wtm-checkpoints-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            let mut folders = Vec::new();
            for i in 0..count {
                let folder = dir.join(format!("repo-{i}"));
                fs::create_dir(&folder).unwrap();
                let folder = folder.to_string_lossy().to_string();
                git(&folder, &["init", "-q"]).unwrap();
                git(&folder, &["config", "user.name", "Test"]).unwrap();
                git(&folder, &["config", "user.email", "test@example.com"]).unwrap();
                fs::write(Path::new(&folder).join("file.txt"), "original").unwrap();
                fs::write(Path::new(&folder).join(".gitignore"), "ignored.txt\n").unwrap();
                git(&folder, &["add", "."]).unwrap();
                git(&folder, &["commit", "-qm", "initial"]).unwrap();
                folders.push(folder);
            }
            let store = Checkpoints::new(&dir.join("app-data"));
            Self {
                dir,
                folders,
                store,
            }
        }
        pub(crate) fn file(&self, repo: usize, name: &str) -> PathBuf {
            Path::new(&self.folders[repo]).join(name)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn restores_all_repositories_and_keeps_user_changes_in_a_recoverable_checkpoint() {
        let fixture = Fixture::new(2);
        fs::write(fixture.file(0, "file.txt"), "staged").unwrap();
        git(&fixture.folders[0], &["add", "file.txt"]).unwrap();
        fs::write(fixture.file(0, "file.txt"), "user's unstaged work").unwrap();
        fs::write(fixture.file(0, "untracked.bin"), [0, 255, 42]).unwrap();
        let checkpoint = fixture.store.capture(&fixture.folders).unwrap();
        let index_before = git(&fixture.folders[0], &["diff", "--cached"]).unwrap();
        fs::write(
            fixture.file(0, "file.txt"),
            "agent's work and new user edits",
        )
        .unwrap();
        fs::remove_file(fixture.file(0, "untracked.bin")).unwrap();
        fs::write(fixture.file(1, "new file.txt"), "new user file").unwrap();
        fs::write(fixture.file(1, "file.txt"), "changed peer").unwrap();
        fs::write(fixture.file(1, "ignored.txt"), "private").unwrap();
        let preview = fixture.store.preview(&checkpoint).unwrap();
        assert_eq!(preview.files.len(), 4);
        let backup = fixture.store.restore(&checkpoint, &preview.token).unwrap();
        assert_eq!(
            fs::read(fixture.file(0, "file.txt")).unwrap(),
            b"user's unstaged work"
        );
        assert_eq!(
            fs::read(fixture.file(0, "untracked.bin")).unwrap(),
            [0, 255, 42]
        );
        assert_eq!(fs::read(fixture.file(1, "file.txt")).unwrap(), b"original");
        assert!(!fixture.file(1, "new file.txt").exists());
        assert_eq!(
            fs::read(fixture.file(1, "ignored.txt")).unwrap(),
            b"private"
        );
        assert_eq!(
            git(&fixture.folders[0], &["diff", "--cached"]).unwrap(),
            index_before
        );
        let undo = fixture.store.preview(&backup).unwrap();
        fixture.store.restore(&backup, &undo.token).unwrap();
        assert_eq!(
            fs::read(fixture.file(1, "new file.txt")).unwrap(),
            b"new user file"
        );
        assert_eq!(
            fs::read(fixture.file(0, "file.txt")).unwrap(),
            b"agent's work and new user edits"
        );
    }

    #[test]
    fn refuses_stale_or_wrong_confirmation_without_touching_files() {
        let fixture = Fixture::new(1);
        let checkpoint = fixture.store.capture(&fixture.folders).unwrap();
        fs::write(fixture.file(0, "file.txt"), "user edits").unwrap();
        let preview = fixture.store.preview(&checkpoint).unwrap();
        fs::write(fixture.file(0, "file.txt"), "newer edits").unwrap();
        assert!(fixture
            .store
            .restore(&checkpoint, &preview.token)
            .unwrap_err()
            .contains("Files changed"));
        assert!(fixture.store.restore(&checkpoint, &checkpoint).is_err());
        assert!(fixture
            .store
            .restore(&preview.token, &preview.token)
            .is_err());
        assert_eq!(
            fs::read(fixture.file(0, "file.txt")).unwrap(),
            b"newer edits"
        );
    }

    #[test]
    fn validates_every_repository_before_restoring_any_and_rejects_changed_head() {
        let fixture = Fixture::new(2);
        fs::write(fixture.file(0, "file.txt"), "local checkpoint baseline").unwrap();
        let checkpoint = fixture.store.capture(&fixture.folders).unwrap();
        fs::write(fixture.file(0, "file.txt"), "keep me").unwrap();
        fs::write(fixture.file(1, "file.txt"), "peer change").unwrap();
        let preview = fixture.store.preview(&checkpoint).unwrap();
        let blob = fixture
            .store
            .dir
            .join("blobs")
            .join(hash(b"local checkpoint baseline"));
        fs::write(&blob, "damaged").unwrap();
        assert!(fixture
            .store
            .restore(&checkpoint, &preview.token)
            .unwrap_err()
            .contains("damaged"));
        assert_eq!(fs::read(fixture.file(0, "file.txt")).unwrap(), b"keep me");
        fs::remove_file(&blob).unwrap();
        assert!(fixture.store.restore(&checkpoint, &preview.token).is_err());
        assert_eq!(
            fs::read(fixture.file(1, "file.txt")).unwrap(),
            b"peer change"
        );
        fs::write(&blob, "local checkpoint baseline").unwrap();
        git(&fixture.folders[1], &["add", "."]).unwrap();
        git(&fixture.folders[1], &["commit", "-qm", "changed head"]).unwrap();
        assert!(fixture.store.restore(&checkpoint, &preview.token).is_err());
        assert_eq!(fs::read(fixture.file(0, "file.txt")).unwrap(), b"keep me");
        assert!(fixture
            .store
            .preview(&checkpoint)
            .unwrap_err()
            .contains("HEAD changed"));
    }

    #[test]
    fn backs_up_previously_untracked_files_that_became_ignored() {
        let fixture = Fixture::new(1);
        fs::write(fixture.file(0, "user.txt"), "before").unwrap();
        let checkpoint = fixture.store.capture(&fixture.folders).unwrap();
        fs::write(fixture.file(0, ".gitignore"), "user.txt\nignored.txt\n").unwrap();
        fs::write(fixture.file(0, "user.txt"), "do not lose this").unwrap();
        let preview = fixture.store.preview(&checkpoint).unwrap();
        fs::write(fixture.file(0, "user.txt"), "latest user edit").unwrap();
        assert!(fixture.store.restore(&checkpoint, &preview.token).is_err());
        assert_eq!(
            fs::read(fixture.file(0, "user.txt")).unwrap(),
            b"latest user edit"
        );
        let preview = fixture.store.preview(&checkpoint).unwrap();
        let backup = fixture.store.restore(&checkpoint, &preview.token).unwrap();
        let undo = fixture.store.preview(&backup).unwrap();
        fixture.store.restore(&backup, &undo.token).unwrap();
        assert_eq!(
            fs::read(fixture.file(0, "user.txt")).unwrap(),
            b"latest user edit"
        );
    }

    #[test]
    fn restores_symlinks_and_permissions_without_following_links() {
        let fixture = Fixture::new(1);
        let outside = fixture.dir.join("outside.txt");
        fs::write(&outside, "outside").unwrap();
        symlink(&outside, fixture.file(0, "link")).unwrap();
        fs::set_permissions(
            fixture.file(0, "file.txt"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let checkpoint = fixture.store.capture(&fixture.folders).unwrap();
        fs::remove_file(fixture.file(0, "link")).unwrap();
        fs::write(fixture.file(0, "link"), "regular file").unwrap();
        fs::set_permissions(
            fixture.file(0, "file.txt"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let preview = fixture.store.preview(&checkpoint).unwrap();
        fixture.store.restore(&checkpoint, &preview.token).unwrap();
        assert_eq!(fs::read_link(fixture.file(0, "link")).unwrap(), outside);
        assert_eq!(
            fs::metadata(fixture.file(0, "file.txt"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert_eq!(fs::read(&outside).unwrap(), b"outside");
        assert!(safe_path(&fixture.folders[0], "../outside.txt").is_err());
        assert!(safe_path(&fixture.folders[0], "link/child").is_err());
    }

    #[test]
    fn mappings_survive_reload_and_are_scoped_to_conversation() {
        let fixture = Fixture::new(1);
        let checkpoint = fixture.store.capture(&fixture.folders).unwrap();
        assert!(fixture
            .store
            .ensure_folders(&checkpoint, &fixture.folders)
            .is_ok());
        assert!(fixture.store.ensure_folders(&checkpoint, &[]).is_err());
        fixture
            .store
            .bind("conversation-a", "user-1", &checkpoint)
            .unwrap();
        let reloaded = Checkpoints::new(&fixture.dir.join("app-data"));
        assert_eq!(reloaded.find("conversation-a", "user-1"), Some(checkpoint));
        assert_eq!(reloaded.find("conversation-b", "user-1"), None);
    }

    #[test]
    fn stores_only_local_deltas_and_reuses_identical_states_without_changing_git() {
        let fixture = Fixture::new(1);
        let unusual = "file\twith\nquotes\".bin";
        fs::write(fixture.file(0, "large.bin"), vec![42; 2 * 1024 * 1024]).unwrap();
        fs::write(fixture.file(0, unusual), [0, 255, 1]).unwrap();
        git(&fixture.folders[0], &["add", "."]).unwrap();
        git(&fixture.folders[0], &["commit", "-qm", "baseline"]).unwrap();
        let index = fs::read(fixture.file(0, ".git/index")).unwrap();
        let refs = git(&fixture.folders[0], &["show-ref"]).unwrap();
        let clean = fixture.store.capture(&fixture.folders).unwrap();
        assert_eq!(fixture.store.capture(&fixture.folders).unwrap(), clean);
        let stored: Checkpoint = fixture.store.read(&format!("{clean}.json")).unwrap();
        assert!(stored.repositories[0].delta);
        assert!(stored.repositories[0].files.is_empty());
        assert_eq!(
            fs::read_dir(fixture.store.dir.join("blobs"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            fixture.store.load(&clean).unwrap().repositories[0]
                .files
                .len(),
            4
        );
        assert_eq!(fs::read(fixture.file(0, ".git/index")).unwrap(), index);
        assert_eq!(git(&fixture.folders[0], &["show-ref"]).unwrap(), refs);

        fs::write(fixture.file(0, "file.txt"), "staged user work").unwrap();
        git(&fixture.folders[0], &["add", "file.txt"]).unwrap();
        fs::write(fixture.file(0, "file.txt"), "unstaged user work").unwrap();
        fs::write(fixture.file(0, "user-new.txt"), "unstaged user work").unwrap();
        fs::remove_file(fixture.file(0, unusual)).unwrap();
        let staged_index = fs::read(fixture.file(0, ".git/index")).unwrap();
        let local = fixture.store.capture(&fixture.folders).unwrap();
        let stored: Checkpoint = fixture.store.read(&format!("{local}.json")).unwrap();
        assert_eq!(
            stored.repositories[0]
                .files
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["file.txt", "user-new.txt"]
        );
        assert_eq!(stored.repositories[0].removed, [unusual]);
        assert_eq!(
            fs::read_dir(fixture.store.dir.join("blobs"))
                .unwrap()
                .count(),
            1
        );
        assert_eq!(fixture.store.capture(&fixture.folders).unwrap(), local);
        assert_eq!(
            fs::read(fixture.file(0, ".git/index")).unwrap(),
            staged_index
        );
        assert_eq!(git(&fixture.folders[0], &["show-ref"]).unwrap(), refs);
    }

    #[test]
    fn untracked_additions_edits_and_deletions_after_preview_invalidate_confirmation() {
        let fixture = Fixture::new(1);
        fs::write(fixture.file(0, "user.txt"), "before").unwrap();
        let checkpoint = fixture.store.capture(&fixture.folders).unwrap();
        fs::write(fixture.file(0, "user.txt"), "current").unwrap();
        for change in 0..3 {
            let preview = fixture.store.preview(&checkpoint).unwrap();
            let metadata_count = fs::read_dir(&fixture.store.dir).unwrap().count();
            let blob_count = fs::read_dir(fixture.store.dir.join("blobs"))
                .unwrap()
                .count();
            match change {
                0 => fs::write(fixture.file(0, "late-user.txt"), "created after preview").unwrap(),
                1 => fs::write(fixture.file(0, "user.txt"), "edited after preview").unwrap(),
                _ => fs::remove_file(fixture.file(0, "late-user.txt")).unwrap(),
            }
            assert!(fixture
                .store
                .restore(&checkpoint, &preview.token)
                .unwrap_err()
                .contains("Files changed"));
            assert_eq!(fs::read(fixture.file(0, "file.txt")).unwrap(), b"original");
            assert_eq!(
                fs::read_dir(&fixture.store.dir).unwrap().count(),
                metadata_count
            );
            assert_eq!(
                fs::read_dir(fixture.store.dir.join("blobs"))
                    .unwrap()
                    .count(),
                blob_count
            );
        }
        assert_eq!(
            fs::read(fixture.file(0, "user.txt")).unwrap(),
            b"edited after preview"
        );
    }

    #[test]
    fn restores_staged_deletions_and_untracked_renames_without_restoring_the_index() {
        let fixture = Fixture::new(1);
        fs::remove_file(fixture.file(0, "file.txt")).unwrap();
        git(&fixture.folders[0], &["add", "-u"]).unwrap();
        fs::write(fixture.file(0, "user-original.txt"), "untracked baseline").unwrap();
        let checkpoint = fixture.store.capture(&fixture.folders).unwrap();
        let index = fs::read(fixture.file(0, ".git/index")).unwrap();
        fs::write(fixture.file(0, "file.txt"), "recreated during turn").unwrap();
        fs::rename(
            fixture.file(0, "user-original.txt"),
            fixture.file(0, "user-renamed.txt"),
        )
        .unwrap();
        let preview = fixture.store.preview(&checkpoint).unwrap();
        assert_eq!(preview.files.len(), 3);
        let backup = fixture.store.restore(&checkpoint, &preview.token).unwrap();
        assert!(!fixture.file(0, "file.txt").exists());
        assert!(!fixture.file(0, "user-renamed.txt").exists());
        assert_eq!(
            fs::read(fixture.file(0, "user-original.txt")).unwrap(),
            b"untracked baseline"
        );
        assert_eq!(fs::read(fixture.file(0, ".git/index")).unwrap(), index);
        let undo = fixture.store.preview(&backup).unwrap();
        fixture.store.restore(&backup, &undo.token).unwrap();
        assert_eq!(
            fs::read(fixture.file(0, "user-renamed.txt")).unwrap(),
            b"untracked baseline"
        );
        assert_eq!(
            fs::read(fixture.file(0, "file.txt")).unwrap(),
            b"recreated during turn"
        );
        assert_eq!(fs::read(fixture.file(0, ".git/index")).unwrap(), index);
    }

    #[test]
    fn preserves_raw_crlf_contents_and_detects_edits_to_assume_unchanged_files() {
        let fixture = Fixture::new(1);
        fs::write(
            fixture.file(0, ".gitattributes"),
            "file.txt text eol=crlf\n",
        )
        .unwrap();
        fs::write(fixture.file(0, "file.txt"), "line\n").unwrap();
        git(&fixture.folders[0], &["add", "."]).unwrap();
        git(
            &fixture.folders[0],
            &["commit", "-qm", "text normalization"],
        )
        .unwrap();
        fs::write(fixture.file(0, "file.txt"), "line\r\n").unwrap();
        assert!(git(&fixture.folders[0], &["diff", "HEAD"])
            .unwrap()
            .is_empty());
        let checkpoint = fixture.store.capture(&fixture.folders).unwrap();
        let captured = fixture.store.load(&checkpoint).unwrap();
        assert!(captured.repositories[0].files["file.txt"]
            .git_blob
            .is_none());
        git(
            &fixture.folders[0],
            &["update-index", "--assume-unchanged", "file.txt"],
        )
        .unwrap();
        let index = fs::read(fixture.file(0, ".git/index")).unwrap();
        fs::write(fixture.file(0, "file.txt"), "user edits\r\n").unwrap();
        assert!(git(&fixture.folders[0], &["diff", "HEAD"])
            .unwrap()
            .is_empty());
        let modified = fixture.store.capture(&fixture.folders).unwrap();
        assert_ne!(modified, checkpoint);
        let preview = fixture.store.preview(&checkpoint).unwrap();
        fixture.store.restore(&checkpoint, &preview.token).unwrap();
        assert_eq!(fs::read(fixture.file(0, "file.txt")).unwrap(), b"line\r\n");
        assert_eq!(fs::read(fixture.file(0, ".git/index")).unwrap(), index);
    }

    #[test]
    fn missing_git_base_objects_abort_before_writing_or_deleting_user_files() {
        let fixture = Fixture::new(1);
        let checkpoint = fixture.store.capture(&fixture.folders).unwrap();
        fs::write(fixture.file(0, "file.txt"), "current user work").unwrap();
        fs::write(fixture.file(0, "new.txt"), "new user work").unwrap();
        let preview = fixture.store.preview(&checkpoint).unwrap();
        let captured = fixture.store.load(&checkpoint).unwrap();
        let object = captured.repositories[0].files["file.txt"]
            .git_blob
            .as_ref()
            .unwrap();
        fs::remove_file(
            fixture
                .file(0, ".git/objects")
                .join(&object[..2])
                .join(&object[2..]),
        )
        .unwrap();
        assert!(fixture
            .store
            .restore(&checkpoint, &preview.token)
            .unwrap_err()
            .contains("Git checkpoint object"));
        assert_eq!(
            fs::read(fixture.file(0, "file.txt")).unwrap(),
            b"current user work"
        );
        assert_eq!(
            fs::read(fixture.file(0, "new.txt")).unwrap(),
            b"new user work"
        );
    }

    #[test]
    fn legacy_full_snapshots_and_confirmation_tokens_remain_restorable() {
        let fixture = Fixture::new(1);
        let current = fixture.store.capture(&fixture.folders).unwrap();
        let mut legacy = fixture.store.load(&current).unwrap();
        for repository in &mut legacy.repositories {
            for file in repository.files.values_mut() {
                let bytes = checkpoint_git::read_blob(
                    &repository.folder,
                    file.git_blob.as_deref().unwrap(),
                )
                .unwrap();
                fs::write(fixture.store.dir.join("blobs").join(&file.blob), bytes).unwrap();
                file.git_blob = None;
            }
        }
        let mut legacy_json = serde_json::to_value(&legacy).unwrap();
        for repository in legacy_json["repositories"].as_array_mut().unwrap() {
            repository.as_object_mut().unwrap().remove("delta");
            repository.as_object_mut().unwrap().remove("removed");
        }
        let legacy_id = uuid::Uuid::new_v4().to_string();
        fixture
            .store
            .save(&format!("{legacy_id}.json"), &legacy_json)
            .unwrap();
        assert!(fixture.store.preview(&legacy_id).unwrap().files.is_empty());
        fs::write(fixture.file(0, "file.txt"), "new user changes").unwrap();
        let backup = fixture.store.capture(&fixture.folders).unwrap();
        fixture
            .store
            .save(&format!("preview-{backup}.json"), &legacy_id)
            .unwrap();
        assert_eq!(fixture.store.restore(&legacy_id, &backup).unwrap(), backup);
        assert_eq!(fs::read(fixture.file(0, "file.txt")).unwrap(), b"original");
    }

    #[test]
    fn identical_backups_have_independent_confirmation_tokens() {
        let fixture = Fixture::new(1);
        let first = fixture.store.capture(&fixture.folders).unwrap();
        fs::write(fixture.file(0, "file.txt"), "second state").unwrap();
        let second = fixture.store.capture(&fixture.folders).unwrap();
        fs::write(fixture.file(0, "file.txt"), "current state").unwrap();
        let preview_first = fixture.store.preview(&first).unwrap();
        let preview_second = fixture.store.preview(&second).unwrap();
        assert_ne!(preview_first.token, preview_second.token);
        assert!(fixture.store.validate(&first, &preview_first.token).is_ok());
        assert!(fixture
            .store
            .validate(&second, &preview_second.token)
            .is_ok());
        assert!(fixture
            .store
            .validate(&first, &preview_second.token)
            .is_err());
    }
}
