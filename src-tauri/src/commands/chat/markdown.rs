//! Markdown documents the agent references in chat, read for the inline preview cards. Relative
//! paths resolve against the linking document's directory, then the task repositories in order;
//! only Markdown files are served. Documents inside the task repositories can be saved back from
//! the preview.

use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::commands::validation::{is_inside_any, resolve_markdown_file};

const MAX_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownDocument {
    pub path: String,
    pub text: String,
    pub editable: bool,
}

pub fn read(
    folders: &[String],
    base: Option<&str>,
    path: &str,
    home: Option<&Path>,
) -> Result<MarkdownDocument, String> {
    let found = resolve_markdown_file(folders, base, path, home)?;
    let size = fs::metadata(&found).map_err(|e| e.to_string())?.len();
    if size > MAX_BYTES {
        return Err(format!("File is too large to preview: {path}"));
    }
    let bytes = fs::read(&found).map_err(|e| e.to_string())?;
    Ok(MarkdownDocument {
        path: found.to_string_lossy().to_string(),
        text: String::from_utf8_lossy(&bytes).to_string(),
        editable: std::str::from_utf8(&bytes).is_ok() && is_inside_any(folders, &found),
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
                std::env::temp_dir().join(format!("wtm-markdown-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir.canonicalize().unwrap())
        }

        fn write(&self, relative: &str, text: &str) -> String {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, text).unwrap();
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
    fn resolves_relative_paths_against_each_folder_in_order() {
        let scratch = Scratch::new("relative");
        let primary = scratch.folder("primary");
        let peer = scratch.folder("peer");
        let expected = scratch.write("peer/docs/plan.md", "# Plan");

        let document = read(&[primary, peer], None, "docs/plan.md", None).unwrap();

        assert_eq!(document.path, expected);
        assert_eq!(document.text, "# Plan");
        assert!(document.editable);
    }

    #[test]
    fn resolves_against_the_linking_document_first() {
        let scratch = Scratch::new("base");
        let repo = scratch.folder("repo");
        let linking = scratch.folder("plans");
        scratch.write("repo/next.md", "repo");
        scratch.write("plans/next.md", "plans");

        let beside = read(std::slice::from_ref(&repo), Some(&linking), "next.md", None).unwrap();
        assert_eq!(beside.text, "plans");
        assert!(!beside.editable);
        assert_eq!(read(&[repo], None, "next.md", None).unwrap().text, "repo");
    }

    #[test]
    fn reads_absolute_and_home_paths() {
        let scratch = Scratch::new("absolute");
        let absolute = scratch.write("notes/a.md", "a");
        scratch.write(".claude/plans/b.markdown", "b");

        let repo = scratch.folder("repo");

        let outside = read(&[repo], None, &absolute, None).unwrap();
        assert_eq!(outside.text, "a");
        assert!(!outside.editable);
        let home = read(&[], None, "~/.claude/plans/b.markdown", Some(&scratch.0)).unwrap();
        assert_eq!(home.text, "b");
        assert!(!home.editable);
    }

    #[test]
    fn rejects_other_files_missing_files_and_markdown_symlinks_to_other_files() {
        let scratch = Scratch::new("rejects");
        let folders = [scratch.folder("repo")];
        scratch.write("repo/secret.txt", "secret");
        std::os::unix::fs::symlink(
            scratch.0.join("repo/secret.txt"),
            scratch.0.join("repo/link.md"),
        )
        .unwrap();

        assert!(read(&folders, None, "secret.txt", None)
            .unwrap_err()
            .starts_with("Not a Markdown"));
        assert!(read(&folders, None, "missing.md", None)
            .unwrap_err()
            .starts_with("File not found"));
        assert!(read(&folders, None, "link.md", None)
            .unwrap_err()
            .starts_with("Not a Markdown"));
    }

    #[test]
    fn a_document_that_is_not_utf8_cannot_be_edited() {
        let scratch = Scratch::new("latin1");
        let repo = scratch.folder("repo");
        let path = scratch.0.join("repo/notes.md");
        fs::write(&path, b"caf\xe9").unwrap();

        let document = read(&[repo], None, "notes.md", None).unwrap();

        assert_eq!(document.text, "caf\u{fffd}");
        assert!(!document.editable);
    }

    #[test]
    fn rejects_documents_too_large_to_preview() {
        let scratch = Scratch::new("large");
        let path = scratch.write("big.md", "");
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();

        assert!(read(&[], None, &path, None)
            .unwrap_err()
            .starts_with("File is too large"));
    }

    #[test]
    fn the_command_reads_documents_off_the_main_thread() {
        let scratch = Scratch::new("command");
        let folder = scratch.folder("repo");
        scratch.write("repo/README.md", "# Readme");

        let document = tauri::async_runtime::block_on(super::super::chat_read_markdown(
            vec![folder],
            None,
            "README.md".into(),
        ))
        .unwrap();

        assert_eq!(document.text, "# Readme");
    }
}
