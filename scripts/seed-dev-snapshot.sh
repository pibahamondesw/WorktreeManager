#!/bin/sh
set -eu

source_id=com.worktreemanager.dev
snapshot_id=com.worktreemanager.dev.snapshot
account=linear-api-keys
app_support="${WTM_APP_SUPPORT_DIR:-$HOME/Library/Application Support}"

if [ ! -f "$app_support/$source_id/store.json" ]; then
  echo "No installed-app state found at $app_support/$source_id/store.json." >&2
  exit 1
fi
mkdir -p "$app_support/$snapshot_id"
cp "$app_support/$source_id/store.json" "$app_support/$snapshot_id/store.json"

if secret_hex=$(security find-generic-password -s "$source_id" -a "$account" -w 2>/dev/null | tr -d '\n' | xxd -p | tr -d '\n') && [ -n "$secret_hex" ]; then
  printf 'add-generic-password -U -s %s -a %s -X %s\n' "$snapshot_id" "$account" "$secret_hex" | security -i >/dev/null
else
  echo "No Linear credentials found for $source_id; the snapshot starts without them." >&2
fi

exec "$@"
