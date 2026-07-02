cask "worktreemanager" do
  version "0.2.0"
  sha256 "dc6ae864f67d626f4b66877762d540161d61110a6077c5ce67e680b429b41eb8"

  url "https://github.com/pibahamondesw/WorktreeManager/releases/download/v#{version}/WorktreeManager_universal.app.tar.gz"
  name "WorktreeManager"
  desc "Desktop worktree manager with Linear and GitHub integration"
  homepage "https://github.com/pibahamondesw/WorktreeManager"

  # The app updates itself via the Tauri updater, so Homebrew should not try to
  # manage upgrades or flag it as outdated.
  auto_updates true
  depends_on macos: :catalina

  app "WorktreeManager.app"

  zap trash: [
    "~/Library/Application Support/com.worktreemanager.dev",
    "~/Library/Caches/com.worktreemanager.dev",
    "~/Library/Preferences/com.worktreemanager.dev.plist",
    "~/Library/Saved Application State/com.worktreemanager.dev.savedState",
  ]
end
