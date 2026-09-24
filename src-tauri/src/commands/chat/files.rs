//! `@` mentions: fuzzy search over the files Git knows about in each task repository, so ignored
//! build output never shows up. Paths in the primary repository are relative; peers are absolute.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

use super::super::git::GIT_ENV_SCRUB;

const LIMIT: usize = 30;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMatch {
    pub path: String,
    pub name: String,
}

fn tracked_files(folder: &str) -> Vec<String> {
    let mut command = Command::new("git");
    command
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .current_dir(folder);
    for key in GIT_ENV_SCRUB {
        command.env_remove(key);
    }
    command
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Subsequence match, favouring hits in the file name, consecutive characters and short paths.
fn score(path: &str, query: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(-(path.len() as i64));
    }
    let lower = path.to_lowercase();
    let name_start = lower.rfind('/').map_or(0, |i| i + 1);
    let mut score = 0i64;
    let mut position = 0;
    let mut previous: Option<usize> = None;
    for wanted in query.to_lowercase().chars() {
        let found = lower[position..].find(wanted)? + position;
        score += 1;
        if found >= name_start {
            score += 3;
        }
        if previous.is_some_and(|p| p + 1 == found) {
            score += 5;
        }
        previous = Some(found);
        position = found + wanted.len_utf8();
    }
    if lower[name_start..].starts_with(&query.to_lowercase()) {
        score += 20;
    }
    Some(score * 100 - path.len() as i64)
}

pub fn search(folders: &[String], query: &str) -> Vec<FileMatch> {
    let query = query.trim();
    let mut matches: Vec<(i64, FileMatch)> = Vec::new();
    for (index, folder) in folders.iter().enumerate() {
        for relative in tracked_files(folder) {
            let Some(score) = score(&relative, query) else {
                continue;
            };
            let path = if index == 0 {
                relative.clone()
            } else {
                Path::new(folder)
                    .join(&relative)
                    .to_string_lossy()
                    .to_string()
            };
            let name = relative.rsplit('/').next().unwrap_or(&relative).to_string();
            matches.push((score, FileMatch { path, name }));
        }
    }
    matches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.path.cmp(&b.1.path)));
    matches.truncate(LIMIT);
    matches.into_iter().map(|(_, m)| m).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn repo(tag: &str, files: &[&str]) -> String {
        let dir = std::env::temp_dir().join(format!("wm-chat-files-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut init = Command::new("git");
        init.args(["init", "-q"]).current_dir(&dir);
        for key in GIT_ENV_SCRUB {
            init.env_remove(key);
        }
        assert!(init.status().unwrap().success());
        fs::write(dir.join(".gitignore"), "build/\n").unwrap();
        for file in files {
            let path = dir.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
        dir.to_string_lossy().to_string()
    }

    #[test]
    fn ranks_file_name_hits_and_skips_ignored_files() {
        let primary = repo(
            "primary",
            &["src/chat/ChatPane.tsx", "src/chart.ts", "build/ChatPane.js"],
        );
        let peer = repo("peer", &["docs/chat.md"]);
        let found = search(&[primary.clone(), peer.clone()], "chatpa");
        assert_eq!(found[0].path, "src/chat/ChatPane.tsx");
        assert!(found.iter().all(|m| !m.path.starts_with("build/")));

        let found = search(&[primary.clone(), peer.clone()], "chat.md");
        assert_eq!(found[0].path, format!("{peer}/docs/chat.md"));
        assert_eq!(found[0].name, "chat.md");
        let _ = fs::remove_dir_all(primary);
        let _ = fs::remove_dir_all(peer);
    }

    #[test]
    fn score_requires_every_query_character_in_order() {
        assert!(score("src/chat.ts", "cht").is_some());
        assert!(score("src/chat.ts", "tch").is_none());
        assert!(score("a/chat.ts", "chat") > score("chat/a.ts", "chat"));
    }
}
