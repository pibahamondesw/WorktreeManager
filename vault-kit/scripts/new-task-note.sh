#!/usr/bin/env bash
# Create (or resolve) the Obsidian task-log note for a worktree. Idempotent:
# an existing note is left untouched and its path printed.
#
#   TASK_LOGS=~/Documents/work-vault/task-logs ./new-task-note.sh
#   ./new-task-note.sh --notes-path <dir> --branch <name> --repo <name> --title <text>
#
# With no --branch, the branch and repo are read from the current git worktree.
set -euo pipefail

notes_path="${TASK_LOGS:-}"
branch=""
repo=""
title=""
workspace=""

while [ $# -gt 0 ]; do
  case "$1" in
    --notes-path) notes_path="${2:-}"; shift 2 ;;
    --branch)     branch="${2:-}"; shift 2 ;;
    --repo)       repo="${2:-}"; shift 2 ;;
    --title)      title="${2:-}"; shift 2 ;;
    --workspace)  workspace="${2:-}"; shift 2 ;;
    -h|--help)    sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [ -z "$notes_path" ]; then
  echo "No task-logs folder. Set TASK_LOGS or pass --notes-path." >&2
  exit 2
fi

worktree_path=""
if [ -z "$branch" ]; then
  if ! worktree_path=$(git rev-parse --show-toplevel 2>/dev/null); then
    echo "Not inside a git worktree. Pass --branch explicitly." >&2
    exit 2
  fi
  branch=$(git rev-parse --abbrev-ref HEAD)
  # The worktree folder is named after the branch, so take the repo name from the
  # main clone: --git-common-dir points at the primary repo's .git.
  if [ -z "$repo" ]; then
    common_dir=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)
    if [ -n "$common_dir" ]; then
      repo=$(basename "$(dirname "$common_dir")")
    else
      repo=$(basename "$worktree_path")
    fi
  fi
fi

slugify() {
  printf '%s' "$1" |
    tr '[:upper:]' '[:lower:]' |
    sed -e 's/[^a-z0-9]\{1,\}/-/g' -e 's/^-*//' -e 's/-*$//'
}

# A Linear-style ID in the branch (abc-123) becomes the filename prefix.
issue_id=$(printf '%s' "$branch" |
  grep -oiE '(^|/)[a-z][a-z0-9]{1,9}-[0-9]+' |
  head -1 |
  sed 's|^/||' |
  tr '[:lower:]' '[:upper:]' || true)

branch_slug=$(slugify "${branch##*/}")

if [ -n "$issue_id" ]; then
  # Drop the issue ID from the slug so it isn't repeated in the filename.
  rest=$(printf '%s' "$branch_slug" | sed "s/^$(slugify "$issue_id")-*//")
  file_name="${issue_id}${rest:+-$rest}.md"
else
  file_name="${branch_slug}.md"
fi

mkdir -p "$notes_path/_archive"

# An existing note wins — including one already archived, and one whose slug
# drifted but whose issue ID matches.
for dir in "$notes_path" "$notes_path/_archive"; do
  if [ -f "$dir/$file_name" ]; then
    printf '%s\n' "$dir/$file_name"
    exit 0
  fi
  if [ -n "$issue_id" ]; then
    existing=$(find "$dir" -maxdepth 1 -name "${issue_id}-*.md" -print -quit 2>/dev/null || true)
    if [ -n "$existing" ]; then
      printf '%s\n' "$existing"
      exit 0
    fi
  fi
done

today=$(date +%Y-%m-%d)
[ -n "$title" ] || title="${issue_id:+$issue_id — }$branch"

{
  echo "---"
  echo "title: \"$title\""
  echo "type: task-log"
  echo "status: active"
  echo "created: $today"
  echo "updated: $today"
  echo "tags: []"
  echo "tickets: [${issue_id}]"
  echo "branch: \"$branch\""
  echo "workspace: \"$workspace\""
  echo "repos: [${repo}]"
  if [ -n "$worktree_path" ]; then
    echo "worktrees:"
    echo "  - repo: ${repo}"
    echo "    path: \"$worktree_path\""
  else
    echo "worktrees: []"
  fi
  echo "prs: []"
  echo "related: []"
  echo "---"
  echo
  echo "## Context"
  echo
  echo "## Decisions"
  echo
  echo "## Learnings"
  echo
  echo "## Log"
} > "$notes_path/$file_name"

printf '%s\n' "$notes_path/$file_name"
