#!/usr/bin/env bash
# archive.sh — archive one or more vault projects
# Sets `status: archived` (and bumps `updated:`) in each <slug>_project.md, then moves
# the whole folder from projects/ to _archive/ (internal structure intact).
# Usage: archive.sh [--dry-run] <project> [<project> ...]
#   <project> is a folder name or any unique substring of it (the YYYY-MM- prefix
#   is optional), e.g. `rcn-2520-bice-cartola-without-webhook` or `compose`.
set -euo pipefail

VAULT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROJECTS_DIR="$VAULT_ROOT/projects"
ARCHIVE_DIR="$VAULT_ROOT/_archive"
TODAY="$(date +%Y-%m-%d)"
USAGE="Usage: archive.sh [--dry-run] <project> [<project> ...]"

# ── argument parsing ─────────────────────────────────────────────────────────
DRY_RUN=0
QUERIES=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    -n|--dry-run) DRY_RUN=1; shift ;;
    -h|--help)    echo "$USAGE"; exit 0 ;;
    -*)
      echo "Unknown option: $1" >&2
      echo "$USAGE" >&2
      exit 1
      ;;
    *) QUERIES+=("$1"); shift ;;
  esac
done

if [[ ${#QUERIES[@]} -eq 0 ]]; then
  echo "Error: at least one project is required." >&2
  echo "$USAGE" >&2
  exit 1
fi

# ── resolve a query to exactly one project folder name ───────────────────────
resolve_project() {
  local query="$1"
  # exact folder-name match wins
  if [[ -d "$PROJECTS_DIR/$query" ]]; then
    echo "$query"
    return 0
  fi
  # otherwise, substring match across active project folders
  local matches=()
  local dir base
  for dir in "$PROJECTS_DIR"/*/; do
    [[ -d "$dir" ]] || continue
    base="$(basename "$dir")"
    if [[ "$base" == *"$query"* ]]; then
      matches+=("$base")
    fi
  done
  if [[ ${#matches[@]} -eq 1 ]]; then
    echo "${matches[0]}"
    return 0
  elif [[ ${#matches[@]} -eq 0 ]]; then
    echo "Error: no active project matches '$query'." >&2
    return 1
  else
    echo "Error: '$query' is ambiguous — matches:" >&2
    printf '  - %s\n' "${matches[@]}" >&2
    return 1
  fi
}

# ── set status: archived (and updated:) in the first frontmatter block ───────
mark_archived() {
  local file="$1"
  local tmp="${file}.tmp.$$"
  awk -v today="$TODAY" '
    BEGIN { fm = 0; sdone = 0; udone = 0 }
    NR == 1 && $0 == "---" { fm = 1; print; next }
    fm == 1 && $0 == "---" { fm = 0; print; next }
    fm == 1 && sdone == 0 && /^status:/  { print "status: archived"; sdone = 1; next }
    fm == 1 && udone == 0 && /^updated:/ { print "updated: " today;  udone = 1; next }
    { print }
  ' "$file" > "$tmp" && mv "$tmp" "$file"
}

# ── resolve everything up front (fail before moving anything) ────────────────
RESOLVED=()
for q in "${QUERIES[@]}"; do
  name="$(resolve_project "$q")" || exit 1
  dest="$ARCHIVE_DIR/$name"
  if [[ -e "$dest" ]]; then
    echo "Error: '$name' already exists in _archive/ — refusing to overwrite." >&2
    exit 1
  fi
  RESOLVED+=("$name")
done

# ── archive each ─────────────────────────────────────────────────────────────
mkdir -p "$ARCHIVE_DIR"
echo ""
for name in "${RESOLVED[@]}"; do
  src="$PROJECTS_DIR/$name"
  # the project overview is the single <slug>_project.md in the folder
  pm=""
  for cand in "$src"/*_project.md; do
    [[ -e "$cand" ]] && { pm="$cand"; break; }
  done

  if [[ $DRY_RUN -eq 1 ]]; then
    echo "  [dry-run] would archive: $name"
    [[ -n "$pm" ]] || echo "            (warning: no <slug>_project.md found)"
    continue
  fi

  if [[ -n "$pm" ]]; then
    mark_archived "$pm"
  else
    echo "  warning: $name has no <slug>_project.md (moving anyway)"
  fi
  mv "$src" "$ARCHIVE_DIR/$name"
  echo "  Archived: $name  →  _archive/$name"
done
echo ""
