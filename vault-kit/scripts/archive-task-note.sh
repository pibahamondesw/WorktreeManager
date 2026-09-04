#!/usr/bin/env bash
# Archive a task-log note: set status/updated in its first frontmatter block and
# move its task folder into _archive/. Legacy flat notes move individually.
# Notes are always preserved. Idempotent; a missing note is fine.
#
#   TASK_LOGS=~/Documents/work-vault/task-logs ./archive-task-note.sh WOR-39-evaluar-obsidian.md
#   ./archive-task-note.sh --notes-path <dir> <file-name> [<file-name> ...]
set -euo pipefail

notes_path="${TASK_LOGS:-}"
files=()

while [ $# -gt 0 ]; do
  case "$1" in
    --notes-path) notes_path="${2:-}"; shift 2 ;;
    -h|--help)    sed -n '2,7p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) files+=("$1"); shift ;;
  esac
done

if [ -z "$notes_path" ]; then
  echo "No task-logs folder. Set TASK_LOGS or pass --notes-path." >&2
  exit 2
fi
if [ ${#files[@]} -eq 0 ]; then
  echo "No note given." >&2
  exit 2
fi

mkdir -p "$notes_path/_archive"
today=$(date +%Y-%m-%d)

for file_name in "${files[@]}"; do
  case "$file_name" in
    */*.md)
      folder="${file_name%%/*}"
      name="${file_name#*/}"
      case "$folder" in ''|_archive|*[!a-zA-Z0-9_-]*) echo "Invalid task folder" >&2; exit 2 ;; esac
      case "$name" in */*|*\\*) echo "Invalid note name" >&2; exit 2 ;; esac
      source="$notes_path/$folder"
      target="$notes_path/_archive/$folder"
      ;;
    *.md)
      case "$file_name" in *\\*) echo "Invalid note name" >&2; exit 2 ;; esac
      source="$notes_path/$file_name"
      target="$notes_path/_archive/$file_name"
      ;;
    *) echo "Expected <task-id>/<note>.md or a legacy <note>.md" >&2; exit 2 ;;
  esac
  src="$notes_path/$file_name"

  if [ ! -f "$src" ]; then
    echo "skip (not found): $file_name" >&2
    continue
  fi
  if [ -e "$target" ]; then
    echo "Archive destination already exists: $target" >&2
    exit 1
  fi

  # Rewrite status/updated only inside the first frontmatter block, so a
  # "status:" mention in the body is left alone.
  awk -v today="$today" '
    NR == 1 && $0 == "---" { print; in_fm = 1; next }
    in_fm && $0 == "---"   { print; in_fm = 0; next }
    in_fm && /^status:/    { print "status: archived"; next }
    in_fm && /^updated:/   { print "updated: " today; next }
    { print }
  ' "$src" > "$src.wm.tmp"

  mv "$src.wm.tmp" "$src"
  mv "$source" "$target"
  printf '%s\n' "$notes_path/_archive/$file_name"
done
