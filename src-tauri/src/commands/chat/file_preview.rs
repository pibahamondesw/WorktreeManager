//! Files the agent links in chat, read for the file preview. Text files can be saved back from the
//! preview.

use std::fs;
use std::path::Path;

use serde::Serialize;

use super::changes::{content, image_mime, FileContent, MAX_IMAGE_BYTES};
use crate::commands::user_file::UserFile;
use crate::commands::validation::resolve_file;

const MAX_TEXT_BYTES: usize = 1024 * 1024;

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewFile {
    pub path: String,
    pub content: FileContent,
}

fn size_limit(path: &str) -> usize {
    if image_mime(path).is_some() {
        MAX_IMAGE_BYTES
    } else {
        MAX_TEXT_BYTES
    }
}

pub fn read(
    folders: &[String],
    base: Option<&str>,
    path: &str,
    home: Option<&Path>,
) -> Result<PreviewFile, String> {
    let found = resolve_file(folders, base, path, home)?;
    let unreadable = |e: std::io::Error| format!("Cannot read {path}: {e}");
    let size = fs::metadata(&found).map_err(unreadable)?.len() as usize;
    let found_path = found.to_string_lossy().to_string();
    if size > size_limit(&found_path) {
        return Err(format!("File is too large to preview: {path}"));
    }
    let bytes = fs::read(&found).map_err(unreadable)?;
    Ok(PreviewFile {
        content: content(&found_path, bytes),
        path: found_path,
    })
}

/// Saves text edited in the preview. Refuses when the file no longer holds the text the edit
/// started from, so concurrent edits are never overwritten.
pub fn write(path: &str, original: &str, text: &str) -> Result<PreviewFile, String> {
    let found = resolve_file(&[], None, path, None)?;
    if fs::read(&found).ok().as_deref() != Some(original.as_bytes()) {
        return Err(format!(
            "{path} changed on disk since you started editing. Reload it and apply your edit again."
        ));
    }
    UserFile::new(&found)
        .unchanged_from(original)
        .replace(text)?;
    let found_path = found.to_string_lossy().to_string();
    Ok(PreviewFile {
        content: content(&found_path, text.as_bytes().to_vec()),
        path: found_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("wtm-preview-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir.canonicalize().unwrap())
        }

        fn write(&self, relative: &str, bytes: &[u8]) -> String {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, bytes).unwrap();
            path.to_string_lossy().to_string()
        }

        fn folder(&self, relative: &str) -> String {
            let path = self.0.join(relative);
            fs::create_dir_all(&path).unwrap();
            path.to_string_lossy().to_string()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn reads_relative_and_absolute_paths_inside_the_task_repositories() {
        let scratch = Scratch::new("inside");
        let folders = [scratch.folder("primary"), scratch.folder("peer")];
        let expected = scratch.write("peer/src/main.rs", b"fn main() {}");

        let relative = read(&folders, None, "src/main.rs", None).unwrap();
        let absolute = read(&folders, None, &expected, None).unwrap();

        assert_eq!(relative.path, expected);
        assert_eq!(
            relative.content,
            FileContent::Text {
                text: "fn main() {}".into()
            }
        );
        assert_eq!(absolute, relative);
    }

    #[test]
    fn reads_absolute_and_home_paths_outside_the_task_repositories() {
        let scratch = Scratch::new("outside");
        let folders = [scratch.folder("repo")];
        let expected = scratch.write("home/.skills/SKILL.txt", b"skill");

        let absolute = read(&folders, None, &expected, None).unwrap();
        let home = read(
            &folders,
            None,
            "~/.skills/SKILL.txt",
            Some(&scratch.0.join("home")),
        )
        .unwrap();

        assert_eq!(absolute.path, expected);
        assert_eq!(
            absolute.content,
            FileContent::Text {
                text: "skill".into()
            }
        );
        assert_eq!(home, absolute);
    }

    #[test]
    fn reads_files_outside_the_task_repositories_through_symlinks() {
        let scratch = Scratch::new("symlink");
        let folders = [scratch.folder("repo")];
        let target = scratch.write("shared/config.txt", b"shared");
        std::os::unix::fs::symlink(&target, scratch.0.join("repo/link.txt")).unwrap();

        assert_eq!(read(&folders, None, "link.txt", None).unwrap().path, target);
    }

    #[test]
    fn reports_missing_files_as_not_found() {
        let scratch = Scratch::new("missing");
        let folders = [scratch.folder("repo")];
        scratch.write("repo/file.txt", b"");

        for path in ["missing.rs", "file.txt/child", "/nowhere/missing.rs"] {
            assert_eq!(
                read(&folders, None, path, None).unwrap_err(),
                format!("File not found: {path}")
            );
        }
    }

    #[test]
    fn reports_why_an_unreadable_file_cannot_be_read() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new("unreadable");
        let folders = [scratch.folder("repo")];
        let path = scratch.write("private/key.txt", b"secret");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();

        let error = read(&folders, None, &path, None).unwrap_err();

        assert_eq!(
            error,
            format!("Cannot read {path}: Permission denied (os error 13)")
        );
    }

    #[test]
    fn reports_why_a_file_in_an_unreadable_directory_cannot_be_read() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new("locked");
        let folders = [scratch.folder("repo")];
        let path = scratch.write("locked/key.txt", b"secret");
        let locked = scratch.0.join("locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

        let error = read(&folders, None, &path, None).unwrap_err();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(
            error,
            format!("Cannot read {path}: Permission denied (os error 13)")
        );
    }

    #[test]
    fn reports_binary_files_without_their_bytes() {
        let scratch = Scratch::new("binary");
        let folders = [scratch.folder("repo")];
        scratch.write("repo/data.bin", &[0, 1, 2]);

        let file = read(&folders, None, "data.bin", None).unwrap();

        assert_eq!(file.content, FileContent::Binary { size: 3 });
    }

    #[test]
    fn keeps_searching_the_task_repositories_past_an_unreadable_candidate() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new("skip-unreadable");
        let folders = [scratch.folder("locked"), scratch.folder("peer")];
        let locked = scratch.0.join("locked/src");
        fs::create_dir_all(&locked).unwrap();
        let expected = scratch.write("peer/src/x.ts", b"x");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

        let found = read(&folders, None, "src/x.ts", None);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(found.unwrap().path, expected);
    }

    #[test]
    fn resolves_against_the_linking_document_before_the_task_repositories() {
        let scratch = Scratch::new("base");
        let folders = [scratch.folder("repo")];
        let expected = scratch.write("repo/src/x.ts", b"x");
        let beside = scratch.write("notes/key", b"note");
        let docs = scratch.folder("repo/docs");
        let notes = scratch.0.join("notes").to_string_lossy().to_string();

        assert_eq!(
            read(&folders, Some(&docs), "../src/x.ts", None)
                .unwrap()
                .path,
            expected
        );
        assert_eq!(
            read(&folders, Some(&notes), "key", None).unwrap().path,
            beside
        );
    }

    #[test]
    fn rejects_text_files_too_large_to_preview_without_reading_them() {
        let scratch = Scratch::new("large");
        let folders = [scratch.folder("repo")];
        let path = scratch.write("repo/big.log", b"");
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_TEXT_BYTES as u64 + 1)
            .unwrap();

        assert!(read(&folders, None, "big.log", None)
            .unwrap_err()
            .starts_with("File is too large"));
    }

    #[test]
    fn previews_images_larger_than_the_text_limit() {
        let scratch = Scratch::new("image");
        let folders = [scratch.folder("repo")];
        let path = scratch.write("repo/shot.png", b"");
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_TEXT_BYTES as u64 + 1)
            .unwrap();

        let file = read(&folders, None, "shot.png", None).unwrap();

        assert!(matches!(file.content, FileContent::Image { .. }));
    }

    #[test]
    fn saves_edited_text_in_place() {
        let scratch = Scratch::new("save");
        let folders = [scratch.folder("repo")];
        let path = scratch.write("repo/src/x.ts", b"const a = 1;\n");

        let saved = write(&path, "const a = 1;\n", "const a = 2;\n").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "const a = 2;\n");
        assert_eq!(saved, read(&folders, None, &path, None).unwrap());
    }

    #[test]
    fn refuses_to_overwrite_a_file_changed_since_the_edit_started() {
        let scratch = Scratch::new("conflict");
        let path = scratch.write("x.ts", b"agent edit\n");

        let error = write(&path, "original\n", "mine\n").unwrap_err();

        assert!(error.contains("changed on disk"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "agent edit\n");
    }

    #[test]
    fn saves_outside_the_task_repositories_through_symlinks() {
        let scratch = Scratch::new("save-outside");
        let target = scratch.write("shared/config.txt", b"old\n");
        let link = scratch.0.join("repo/link.txt");
        scratch.folder("repo");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let saved = write(&link.to_string_lossy(), "old\n", "new\n").unwrap();

        assert_eq!(saved.path, target);
        assert_eq!(fs::read_to_string(&target).unwrap(), "new\n");
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn refuses_to_save_a_missing_file() {
        let scratch = Scratch::new("save-missing");
        let path = scratch.0.join("gone.txt").to_string_lossy().to_string();

        assert_eq!(
            write(&path, "", "new").unwrap_err(),
            format!("File not found: {path}")
        );
        assert!(!Path::new(&path).exists());
    }

    #[test]
    fn the_command_saves_files_off_the_main_thread() {
        let scratch = Scratch::new("save-command");
        let path = scratch.write("a.txt", b"a");

        let saved = tauri::async_runtime::block_on(super::super::chat_write_file(
            path.clone(),
            "a".into(),
            "b".into(),
        ))
        .unwrap();

        assert_eq!(saved.content, FileContent::Text { text: "b".into() });
    }

    #[test]
    fn the_command_reads_files_off_the_main_thread() {
        let scratch = Scratch::new("command");
        let folder = scratch.folder("repo");
        let docs = scratch.folder("repo/docs");
        scratch.write("repo/src/lib.rs", b"pub fn a() {}");

        let file = tauri::async_runtime::block_on(super::super::chat_read_file(
            vec![folder],
            Some(docs),
            "../src/lib.rs".into(),
        ))
        .unwrap();

        assert!(file.path.ends_with("repo/src/lib.rs"));
    }
}
