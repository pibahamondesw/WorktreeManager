//! Markdown documents the agent references in chat, read for the inline preview cards. Relative
//! paths resolve against the task repositories in order; only Markdown files are served.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

const MAX_BYTES: u64 = 2 * 1024 * 1024;
const EXTENSIONS: [&str; 3] = ["md", "markdown", "mdx"];

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownDocument {
    pub path: String,
    pub text: String,
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
}

fn candidates(folders: &[String], path: &str, home: Option<&Path>) -> Vec<PathBuf> {
    if let Some(rest) = path.strip_prefix("~/") {
        return home.map(|home| home.join(rest)).into_iter().collect();
    }
    let path = Path::new(path);
    if path.is_absolute() {
        return vec![path.to_path_buf()];
    }
    folders
        .iter()
        .map(|folder| Path::new(folder).join(path))
        .collect()
}

pub fn read(
    folders: &[String],
    path: &str,
    home: Option<&Path>,
) -> Result<MarkdownDocument, String> {
    if !is_markdown(Path::new(path)) {
        return Err(format!("Not a Markdown file: {path}"));
    }
    let found = candidates(folders, path, home)
        .into_iter()
        .find_map(|candidate| candidate.canonicalize().ok().filter(|real| real.is_file()))
        .ok_or_else(|| format!("File not found: {path}"))?;
    if !is_markdown(&found) {
        return Err(format!("Not a Markdown file: {path}"));
    }
    let size = fs::metadata(&found).map_err(|e| e.to_string())?.len();
    if size > MAX_BYTES {
        return Err(format!("File is too large to preview: {path}"));
    }
    let bytes = fs::read(&found).map_err(|e| e.to_string())?;
    Ok(MarkdownDocument {
        path: found.to_string_lossy().to_string(),
        text: String::from_utf8_lossy(&bytes).to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let document = read(&[primary, peer], "docs/plan.md", None).unwrap();

        assert_eq!(document.path, expected);
        assert_eq!(document.text, "# Plan");
    }

    #[test]
    fn reads_absolute_and_home_paths() {
        let scratch = Scratch::new("absolute");
        let absolute = scratch.write("notes/a.md", "a");
        scratch.write(".claude/plans/b.markdown", "b");

        assert_eq!(read(&[], &absolute, None).unwrap().text, "a");
        let home = read(&[], "~/.claude/plans/b.markdown", Some(&scratch.0)).unwrap();
        assert_eq!(home.text, "b");
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

        assert!(read(&folders, "secret.txt", None)
            .unwrap_err()
            .starts_with("Not a Markdown"));
        assert!(read(&folders, "missing.md", None)
            .unwrap_err()
            .starts_with("File not found"));
        assert!(read(&folders, "link.md", None)
            .unwrap_err()
            .starts_with("Not a Markdown"));
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

        assert!(read(&[], &path, None)
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
            "README.md".into(),
        ))
        .unwrap();

        assert_eq!(document.text, "# Readme");
    }
}
