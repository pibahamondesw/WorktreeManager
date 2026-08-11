#!/usr/bin/env bash
# Archive a task-log note: set status/updated in its first frontmatter block and
# move it into _archive/. Idempotent; a missing note is not an error.
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
  file_name=$(basename "$file_name")
  src="$notes_path/$file_name"

  if [ ! -f "$src" ]; then
    echo "skip (not found): $file_name" >&2
    continue
  fi
  if [ -f "$notes_path/_archive/$file_name" ]; then
    echo "skip (already archived): $file_name" >&2
    continue
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
  mv "$src" "$notes_path/_archive/$file_name"
  printf '%s\n' "$notes_path/_archive/$file_name"
done
