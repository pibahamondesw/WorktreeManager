# Homebrew distribution

WorktreeManager is installed through a **custom Homebrew tap** so users can get a
prebuilt app with one command instead of building from source:

```sh
brew install pibahamondesw/tap/worktreemanager
```

## One-time setup (maintainer)

1. **Create the tap repo.** On GitHub, create a public repo named
   **`pibahamondesw/homebrew-tap`** (the `homebrew-` prefix is required; the
   generic `-tap` name lets it hold casks for other apps later — one
   `Casks/<app>.rb` per app).

2. **Seed the cask.** Copy `homebrew/worktreemanager.rb` from this repo into the
   tap at `Casks/worktreemanager.rb`. After the first real release, update its
   `version` and `sha256` to match (the checksum is
   `shasum -a 256 WorktreeManager.app.tar.gz`). From then on the release workflow
   keeps it up to date automatically.

3. **Enable auto-bump (optional but recommended).** Create a fine-grained
   Personal Access Token with **Contents: read/write** scoped to
   `pibahamondesw/homebrew-tap`, and add it to the **WorktreeManager** repo as a
   secret named `HOMEBREW_TAP_TOKEN`. The `update-tap` job in
   `.github/workflows/release.yml` then rewrites the cask's `version`/`sha256`
   and pushes it on every tagged release. If the secret is absent, that job
   no-ops and you bump the cask by hand.

## Verifying the cask

```sh
brew audit --cask --new Casks/worktreemanager.rb
brew style Casks/worktreemanager.rb
brew install --cask ./Casks/worktreemanager.rb   # local install test
```

## Why `auto_updates true`?

The app self-updates via the Tauri updater. `auto_updates true` tells Homebrew
not to fight that (it won't report the app as outdated or try to reinstall it).
Running `brew upgrade` still works after a cask bump for anyone who prefers it.

## Why a `.app.tar.gz` and not a `.dmg`?

The release distributes the app as a gzipped tarball rather than a DMG. The DMG
bundler styles the disk-image window via AppleScript/Finder automation, which is
unreliable in headless CI. The tarball is what the Tauri updater already produces
and is rock-solid to build. Homebrew installs it identically to a DMG, so users
notice no difference. A DMG can be added later (set `bundle.targets` to
`["app", "dmg"]` and point the cask `url` at the DMG) once it's confirmed to build
on the release runner.
