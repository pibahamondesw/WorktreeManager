#!/bin/sh
set -eu

if [ -n "${WTM_DEV_SIGNING_IDENTITY:-}" ]; then
  if [ "$WTM_DEV_SIGNING_IDENTITY" = "-" ]; then
    echo "Shared development requires a certificate, not an ad-hoc signature." >&2
    exit 1
  fi
  codesign --force --sign "$WTM_DEV_SIGNING_IDENTITY" \
    --identifier com.worktreemanager.dev --timestamp=none "$1" || {
    echo "Create the code-signing certificate described in README.md, or set WTM_DEV_SIGNING_IDENTITY to an existing identity." >&2
    exit 1
  }
  codesign --verify --strict "$1"
fi

exec "$@"
