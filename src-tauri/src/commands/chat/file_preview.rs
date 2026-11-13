//! Files the agent links in chat, read for the file preview. Unlike Markdown documents, only files
//! inside the task repositories are served, even when the path resolves against the directory of
//! the document holding the link.

use std::fs;
use std::path::Path;

use serde::Serialize;

use super::changes::{content, image_mime, FileContent, MAX_IMAGE_BYTES};
use super::markdown::candidates;

const MAX_TEXT_BYTES: usize = 1024 * 1024;

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewFile {
    pub path: String,
    pub content: FileContent,
}

fn is_inside_any(folders: &[String], path: &Path) -> bool {
    folders
        .iter()
        .filter_map(|folder| Path::new(folder).canonicalize().ok())
        .any(|folder| path.starts_with(folder))
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
    let roots: Vec<String> = base
        .map(String::from)
        .into_iter()
        .chain(folders.iter().cloned())
        .collect();
    let found = candidates(&roots, path, home)
        .into_iter()
        .filter_map(|candidate| candidate.canonicalize().ok())
        .find(|real| real.is_file() && is_inside_any(folders, real))
        .ok_or_else(|| format!("File not found in the task repositories: {path}"))?;
    let size = fs::metadata(&found).map_err(|e| e.to_string())?.len() as usize;
    let found_path = found.to_string_lossy().to_string();
    if size > size_limit(&found_path) {
        return Err(format!("File is too large to preview: {path}"));
    }
    let bytes = fs::read(&found).map_err(|e| e.to_string())?;
    Ok(PreviewFile {
        content: content(&found_path, bytes),
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
    fn rejects_files_outside_the_task_repositories_including_through_symlinks() {
        let scratch = Scratch::new("outside");
        let folders = [scratch.folder("repo")];
        let secret = scratch.write("secret.txt", b"secret");
        std::os::unix::fs::symlink(&secret, scratch.0.join("repo/link.txt")).unwrap();

        for path in [secret.as_str(), "../secret.txt", "link.txt", "missing.rs"] {
            assert!(read(&folders, None, path, None)
                .unwrap_err()
                .starts_with("File not found"));
        }
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
    fn resolves_against_the_linking_document_without_serving_files_outside_the_task() {
        let scratch = Scratch::new("base");
        let folders = [scratch.folder("repo")];
        let expected = scratch.write("repo/src/x.ts", b"x");
        scratch.write("notes/key", b"secret");
        let docs = scratch.folder("repo/docs");
        let notes = scratch.0.join("notes").to_string_lossy().to_string();

        assert_eq!(
            read(&folders, Some(&docs), "../src/x.ts", None)
                .unwrap()
                .path,
            expected
        );
        assert!(read(&folders, Some(&notes), "key", None)
            .unwrap_err()
            .starts_with("File not found"));
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
