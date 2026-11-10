//! `@` mentions: fuzzy search over the files Git knows about in each task repository, so ignored
//! build output never shows up. Paths in the primary repository are relative; peers are absolute.
//! Absolute and `~/` queries browse the filesystem instead, one directory at a time.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::super::git::git_command;

const LIMIT: usize = 30;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMatch {
    pub path: String,
    pub name: String,
}

fn tracked_files(folder: &str) -> Vec<String> {
    git_command()
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .current_dir(folder)
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

pub fn search(folders: &[String], query: &str, home: Option<&Path>) -> Vec<FileMatch> {
    if let Some(found) = browse(query.trim(), home) {
        return found;
    }
    let listings: Vec<(&str, Vec<String>)> = folders
        .iter()
        .map(|folder| (folder.as_str(), tracked_files(folder)))
        .collect();
    rank(&listings, query)
}

fn rank(listings: &[(&str, Vec<String>)], query: &str) -> Vec<FileMatch> {
    let query = query.trim();
    let mut matches: Vec<(i64, FileMatch)> = Vec::new();
    for (index, (folder, files)) in listings.iter().enumerate() {
        for relative in files {
            let Some(score) = score(relative, query) else {
                continue;
            };
            let path = if index == 0 {
                relative.clone()
            } else {
                Path::new(folder)
                    .join(relative)
                    .to_string_lossy()
                    .to_string()
            };
            let name = relative.rsplit('/').next().unwrap_or(relative).to_string();
            matches.push((score, FileMatch { path, name }));
        }
    }
    matches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.path.cmp(&b.1.path)));
    matches.truncate(LIMIT);
    matches.into_iter().map(|(_, m)| m).collect()
}

fn browsed_directory(typed: &str, home: Option<&Path>) -> Option<PathBuf> {
    match typed.strip_prefix("~/") {
        Some(rest) => home.map(|home| home.join(rest)),
        None => Some(PathBuf::from(typed)),
    }
}

fn browse(query: &str, home: Option<&Path>) -> Option<Vec<FileMatch>> {
    if !query.starts_with('/') && !query.starts_with("~/") {
        return None;
    }
    let split = query.rfind('/')? + 1;
    let (typed_directory, prefix) = query.split_at(split);
    let Some(entries) =
        browsed_directory(typed_directory, home).and_then(|directory| fs::read_dir(directory).ok())
    else {
        return Some(Vec::new());
    };
    let wanted = prefix.to_lowercase();
    let mut found: Vec<FileMatch> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let hidden = name.starts_with('.') && !prefix.starts_with('.');
            if hidden || !name.to_lowercase().starts_with(&wanted) {
                return None;
            }
            let name = if entry.path().is_dir() {
                format!("{name}/")
            } else {
                name
            };
            Some(FileMatch {
                path: format!("{typed_directory}{name}"),
                name,
            })
        })
        .collect();
    found.sort_by_key(|entry| entry.name.to_lowercase());
    found.truncate(LIMIT);
    Some(found)
}

/// `@path` and `@"path with spaces"` tokens, in order.
pub fn mentioned_paths(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find('@') {
        let starts_token = at == 0 || rest[..at].ends_with(char::is_whitespace);
        rest = &rest[at + 1..];
        if !starts_token {
            continue;
        }
        let path = match rest.strip_prefix('"') {
            Some(quoted) => match quoted.find('"') {
                Some(end) => {
                    rest = &quoted[end + 1..];
                    &quoted[..end]
                }
                None => continue,
            },
            None => {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                let (path, after) = rest.split_at(end);
                rest = after;
                path
            }
        };
        if !path.is_empty() {
            paths.push(path.to_string());
        }
    }
    paths
}

pub fn resolve_mention(path: &str, cwd: &Path, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => cwd.join(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn ranks_file_name_hits_and_prefixes_peer_paths() {
        let listings = [
            (
                "/primary",
                files(&["src/chat/ChatPane.tsx", "src/chart.ts"]),
            ),
            ("/peer", files(&["docs/chat.md"])),
        ];

        let found = rank(&listings, "chatpa");
        assert_eq!(found[0].path, "src/chat/ChatPane.tsx");

        let found = rank(&listings, "chat.md");
        assert_eq!(found[0].path, "/peer/docs/chat.md");
        assert_eq!(found[0].name, "chat.md");
    }

    #[test]
    fn score_requires_every_query_character_in_order() {
        assert!(score("src/chat.ts", "cht").is_some());
        assert!(score("src/chat.ts", "tch").is_none());
        assert!(score("a/chat.ts", "chat") > score("chat/a.ts", "chat"));
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wtm-files-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("Docs")).unwrap();
        fs::write(dir.join("design.pdf"), "").unwrap();
        fs::write(dir.join("deploy.sh"), "").unwrap();
        fs::write(dir.join(".secret"), "").unwrap();
        dir
    }

    #[test]
    fn absolute_queries_browse_the_typed_directory() {
        let dir = scratch_dir("absolute");
        let typed = format!("{}/", dir.display());

        let found = search(&[], &format!("{typed}de"), None);
        let names: Vec<&str> = found.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["deploy.sh", "design.pdf"]);
        assert_eq!(found[1].path, format!("{typed}design.pdf"));

        let found = search(&[], &typed, None);
        let names: Vec<&str> = found.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["deploy.sh", "design.pdf", "Docs/"]);

        let found = search(&[], &format!("{typed}.s"), None);
        assert_eq!(found[0].name, ".secret");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn relative_queries_search_the_task_repositories() {
        assert!(search(&[], "chat", None).is_empty());
    }

    #[test]
    fn home_queries_keep_the_tilde_in_inserted_paths() {
        let home = scratch_dir("home");

        let found = search(&[], "~/Do", Some(&home));
        assert_eq!(
            found,
            [FileMatch {
                path: "~/Docs/".into(),
                name: "Docs/".into()
            }]
        );
        assert!(search(&[], "~/missing/", Some(&home)).is_empty());
        assert!(search(&[], "~/Do", None).is_empty());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn finds_plain_and_quoted_mentions_at_token_starts() {
        assert_eq!(
            mentioned_paths(r#"read @src/a.rs and @"/Users/me/My File.pdf", not me@x.com @"#),
            ["src/a.rs", "/Users/me/My File.pdf"]
        );
        assert_eq!(mentioned_paths(r#"@"unterminated"#), Vec::<String>::new());
    }

    #[test]
    fn resolves_mentions_against_the_cwd_or_home() {
        let cwd = Path::new("/wt/a");
        let home = Path::new("/Users/me");
        assert_eq!(
            resolve_mention("src/a.rs", cwd, Some(home)),
            Path::new("/wt/a/src/a.rs")
        );
        assert_eq!(
            resolve_mention("/tmp/x", cwd, Some(home)),
            Path::new("/tmp/x")
        );
        assert_eq!(
            resolve_mention("~/x.pdf", cwd, Some(home)),
            Path::new("/Users/me/x.pdf")
        );
    }
}
