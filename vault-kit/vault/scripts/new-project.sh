#!/usr/bin/env bash
# new-project.sh — scaffold a new vault project
# Usage: new-project.sh <slug> [--tickets FIN-1,FIN-2] [--linear URL]
set -euo pipefail

VAULT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROJECTS_DIR="$VAULT_ROOT/projects"
TEMPLATES_DIR="$VAULT_ROOT/templates"
TODAY="$(date +%Y-%m)"

# ── argument parsing ─────────────────────────────────────────────────────────
SLUG=""
TICKETS=""
LINEAR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tickets) TICKETS="$2"; shift 2 ;;
    --linear)  LINEAR="$2";  shift 2 ;;
    -*)
      echo "Unknown option: $1" >&2
      echo "Usage: new-project.sh <slug> [--tickets FIN-1,FIN-2] [--linear URL]" >&2
      exit 1
      ;;
    *)
      if [[ -z "$SLUG" ]]; then
        SLUG="$1"
      else
        echo "Unexpected argument: $1" >&2
        exit 1
      fi
      shift
      ;;
  esac
done

if [[ -z "$SLUG" ]]; then
  echo "Error: slug is required." >&2
  echo "Usage: new-project.sh <slug> [--tickets FIN-1,FIN-2] [--linear URL]" >&2
  exit 1
fi

# ── derive folder name ───────────────────────────────────────────────────────
DIR_NAME="${TODAY}-${SLUG}"
PROJECT_DIR="$PROJECTS_DIR/$DIR_NAME"

if [[ -d "$PROJECT_DIR" ]]; then
  echo "Error: project already exists at $PROJECT_DIR" >&2
  exit 1
fi

# ── build YAML ticket list ───────────────────────────────────────────────────
if [[ -n "$TICKETS" ]]; then
  TICKET_LIST="["
  IFS=',' read -ra IDS <<< "$TICKETS"
  for ID in "${IDS[@]}"; do
    TICKET_LIST+="$ID, "
  done
  TICKET_LIST="${TICKET_LIST%, }]"
else
  TICKET_LIST="[]"
fi

# ── create structure ─────────────────────────────────────────────────────────
mkdir -p "$PROJECT_DIR"/{investigations,plans,decisions,documents}

# ── populate <slug>_project.md from template ─────────────────────────────────
TEMPLATE="$TEMPLATES_DIR/project.md"
TARGET="$PROJECT_DIR/${SLUG}_project.md"
FULL_DATE="$(date +%Y-%m-%d)"
TITLE="$(echo "$SLUG" | sed 's/-/ /g' | awk '{for(i=1;i<=NF;i++) $i=toupper(substr($i,1,1)) substr($i,2)} 1')"

sed \
  -e "s/{{title}}/$TITLE/g" \
  -e "s/{{date}}/$FULL_DATE/g" \
  "$TEMPLATE" > "$TARGET"

# patch tickets and linear_project in frontmatter
sed -i '' \
  -e "s/^tickets: \[\]/tickets: $TICKET_LIST/" \
  -e "s|^linear_project: \"\"|linear_project: \"$LINEAR\"|" \
  "$TARGET"

# ── done ─────────────────────────────────────────────────────────────────────
echo ""
echo "  Project created: $PROJECT_DIR"
echo ""
echo "  Subfolders:"
echo "    investigations/   — research notes (copy + rename templates/investigation.md)"
echo "    plans/            — agent plans    (copy + rename templates/plan.md)"
echo "    decisions/        — task decisions (copy + rename templates/decision.md)"
echo "    documents/        — images, PDFs, references"
echo ""
echo "  Edit ${SLUG}_project.md to fill in goal, context, owner, and repos."
echo ""
