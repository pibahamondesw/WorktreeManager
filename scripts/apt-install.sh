#!/bin/sh
set -eu

sources="${APT_SOURCES:-/etc/apt/sources.list.d/ubuntu.sources}"

update_package_lists() {
  sudo apt-get update --error-on=any
}

if ! update_package_lists; then
  if ! grep -q 'mirror+file:' "$sources" 2>/dev/null; then
    echo "Could not download every package list; see the apt errors above." >&2
    exit 1
  fi
  echo "Retrying with archive.ubuntu.com instead of the runner's mirror list." >&2
  sed 's|mirror+file:[^ ]*|http://archive.ubuntu.com/ubuntu/|' "$sources" | sudo tee "$sources.tmp" >/dev/null
  sudo mv "$sources.tmp" "$sources"
  if ! update_package_lists; then
    echo "Could not download every package list, even from archive.ubuntu.com; see the apt errors above." >&2
    exit 1
  fi
fi

sudo apt-get install -y -o Acquire::Retries=3 "$@"
