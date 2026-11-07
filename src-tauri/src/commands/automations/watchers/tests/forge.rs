use super::*;

/// A stand-in for `gh` that records its arguments, input and working directory.
fn fake_gh(dir: &Path, graphql_answer: &str) -> String {
    let path = dir.join("gh");
    let d = dir.display();
    executable(
        &path,
        &format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> "{d}/calls"
case "$1 $2" in
  "api graphql") cat > "{d}/input"; printf '%s' '{graphql_answer}' ;;
  "pr list") pwd > "{d}/cwd"; echo '[{{"url":"https://github.com/o/r/pull/7"}}]' ;;
  "pr comment") ;;
  *) exit 1 ;;
esac
"#
        ),
    );
    path.to_string_lossy().into_owned()
}

#[test]
fn cli_forge_asks_gh_for_graphql_branch_prs_and_comments() {
    let dir = TempDir::new("gh");
    let forge = CliWatchForge::with(&fake_gh(&dir.0, r#"{"data":{"ok":true}}"#));

    let answer = forge
        .graphql("query { ok }", json!({ "number": 7 }))
        .unwrap();
    assert_eq!(answer, json!({ "data": { "ok": true } }));
    let input: Value =
        serde_json::from_str(&fs::read_to_string(dir.0.join("input")).unwrap()).unwrap();
    assert_eq!(
        input,
        json!({ "query": "query { ok }", "variables": { "number": 7 } })
    );
    assert_eq!(
        forge.branch_pull_request(&dir.0, "wtm/unit").unwrap(),
        Some(PR_URL.to_string())
    );
    assert_eq!(
        fs::read_to_string(dir.0.join("cwd")).unwrap().trim(),
        dir.0.to_string_lossy()
    );
    forge.comment(PR_URL, "review please").unwrap();
    let calls = fs::read_to_string(dir.0.join("calls")).unwrap();
    assert_eq!(
        calls.lines().collect::<Vec<_>>(),
        [
            "api graphql --hostname github.com --input -",
            "pr list --head wtm/unit --state all --limit 1 --json url",
            &format!("pr comment {PR_URL} --body review please"),
        ]
    );
}

#[test]
fn cli_forge_reports_gh_failures_and_unreadable_answers() {
    let dir = TempDir::new("gh-bad");
    let forge = CliWatchForge::with(&fake_gh(&dir.0, "not json"));

    assert_eq!(
        forge.graphql("q", json!({})).unwrap_err(),
        "Invalid GitHub response"
    );
    let failing = CliWatchForge::with("false");
    assert!(failing.comment(PR_URL, "x").is_err());
    let listing = TempDir::new("gh-list");
    let lister = dir.0.join("lister");
    executable(&lister, "#!/bin/sh\necho nope\n");
    let forge = CliWatchForge::with(&lister.to_string_lossy());
    assert_eq!(
        forge.branch_pull_request(&listing.0, "b").unwrap_err(),
        "Invalid GitHub response"
    );
}

/// A remote whose `main` has a base commit, and a clone of it that commits as WTM.
fn remote_and_clone() -> (TempDir, TempDir) {
    let remote = TempDir::new("remote");
    git(&remote.0, &["init", "-q", "--bare", "-b", "main"]);
    let clone = TempDir::new("clone");
    git(&clone.0, &["init", "-q", "-b", "main"]);
    git(&clone.0, &["config", "user.email", "wtm@example.com"]);
    git(&clone.0, &["config", "user.name", "WTM"]);
    git(
        &clone.0,
        &["remote", "add", "origin", &remote.0.to_string_lossy()],
    );
    commit(&clone.0, "a.txt", "one\n");
    git(&clone.0, &["push", "-q", "origin", "main"]);
    (remote, clone)
}

fn commit(dir: &Path, file: &str, content: &str) -> String {
    fs::write(dir.join(file), content).unwrap();
    git(dir, &["add", file]);
    git(dir, &["commit", "-q", "-m", file]);
    git(dir, &["rev-parse", "HEAD"])
}

/// Commits on a branch off `main`, published as PR `number`'s head.
fn pull_request(clone: &Path, number: u64, files: &[(&str, &str)]) -> String {
    git(
        clone,
        &["checkout", "-q", "-B", &format!("pr{number}"), "main"],
    );
    let mut head = String::new();
    for (file, content) in files {
        head = commit(clone, file, content);
    }
    let refspec = format!("HEAD:refs/pull/{number}/head");
    git(clone, &["push", "-q", "origin", &refspec]);
    git(clone, &["checkout", "-q", "main"]);
    head
}

#[test]
fn integration_is_ancestry_or_a_patch_already_on_the_base() {
    let (_remote, clone) = remote_and_clone();
    let dir = &clone.0;
    let forge = CliWatchForge::default();
    let squashed = pull_request(dir, 1, &[("b.txt", "two\n"), ("c.txt", "three\n")]);
    let pending = pull_request(dir, 2, &[("d.txt", "four\n")]);
    let conflicting = pull_request(dir, 3, &[("a.txt", "other\n")]);
    let ancestor = pull_request(dir, 4, &[("e.txt", "five\n")]);
    git(dir, &["merge", "-q", "--ff-only", &ancestor]);
    git(dir, &["checkout", "-q", &squashed, "--", "b.txt", "c.txt"]);
    git(dir, &["commit", "-q", "-m", "Squashed by bors"]);
    commit(dir, "a.txt", "one more\n");
    git(dir, &["push", "-q", "origin", "main"]);

    assert_eq!(forge.integrated(dir, 1, &squashed, "main"), Ok(true));
    assert_eq!(forge.integrated(dir, 4, &ancestor, "main"), Ok(true));
    assert_eq!(forge.integrated(dir, 2, &pending, "main"), Ok(false));
    assert_eq!(forge.integrated(dir, 3, &conflicting, "main"), Ok(false));
    let unknown = forge.integrated(dir, 1, "f00d", "main").unwrap_err();
    assert!(unknown.starts_with("git merge-base"), "{unknown}");
    assert_eq!(
        forge.integrated(dir, 1, &squashed, "gone"),
        Err("Could not fetch gone and PR #1 from origin".into())
    );
}

#[test]
fn local_git_failures_name_the_command() {
    let dir = TempDir::new("plain");
    let error = super::super::forge::git(&dir.0, &["rev-parse", "HEAD"]).unwrap_err();
    assert!(error.starts_with("git rev-parse"), "{error}");
}
