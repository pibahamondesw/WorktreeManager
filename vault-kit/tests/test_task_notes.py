import subprocess
import tempfile
import unittest
from pathlib import Path


SCRIPTS = Path(__file__).resolve().parents[1] / "scripts"


class TaskNotesTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="wm-task-notes-")
        self.addCleanup(self.temp.cleanup)
        self.notes = Path(self.temp.name) / "task logs"
        self.notes.mkdir()

    def run_script(self, script, *args, cwd=None, check=True):
        return subprocess.run(
            ["bash", str(SCRIPTS / script), "--notes-path", str(self.notes), *args],
            cwd=cwd, text=True, capture_output=True, check=check,
        )

    def create(self, task_id):
        result = self.run_script(
            "new-task-note.sh", "--task-id", task_id,
            "--branch", "dev/wor-56-notes", "--repo", "repo",
        )
        return Path(result.stdout.strip())

    def test_repeated_names_keep_separate_notes_and_attachments(self):
        first = self.create("first")
        second = self.create("second")
        attachment = first.parent / "attachment.txt"
        attachment.write_text("keep this")
        self.run_script("archive-task-note.sh", str(first.relative_to(self.notes)))
        self.assertFalse(first.exists())
        archived = self.notes / "_archive" / "first" / first.name
        self.assertIn("status: archived", archived.read_text())
        self.assertEqual((archived.parent / attachment.name).read_text(), "keep this")
        self.assertIn("status: active", second.read_text())
        self.assertEqual(self.create("first"), archived)
        self.assertNotEqual(self.create("third"), archived)

    def test_empty_and_legacy_notes_are_archived(self):
        for name, content in [("empty.md", ""), ("headings.md", "# Decision\n"),
                              ("properties.md", "---\nprs: [important]\n---\n")]:
            note = self.notes / name
            note.write_text(content)
            self.run_script("archive-task-note.sh", name)
            self.assertFalse(note.exists())
            self.assertEqual((self.notes / "_archive" / name).read_text(), content)
            self.run_script("archive-task-note.sh", name)

    def test_existing_archive_is_never_overwritten(self):
        note = self.create("first")
        archived = self.notes / "_archive" / "first" / note.name
        archived.parent.mkdir(parents=True)
        archived.write_text("history")
        result = self.run_script("archive-task-note.sh", str(note.relative_to(self.notes)), check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(archived.read_text(), "history")
        self.assertIn("status: active", note.read_text())

    def test_resolver_prefers_active_worktree_match_and_rejects_ambiguity(self):
        repo = Path(self.temp.name).resolve() / "repo"
        subprocess.run(["git", "init", "-q", str(repo)], check=True)
        subprocess.run(["git", "-C", str(repo), "-c", "user.name=Test", "-c",
                        "user.email=test@example.test", "commit", "-qm", "Initial", "--allow-empty"], check=True)
        content = f'---\nworktrees:\n  - repo: repo\n    path: "{repo}"\n---\n'
        active = self.notes / "current" / "renamed-note.md"
        archived = self.notes / "_archive" / "old" / "renamed-note.md"
        for note in [active, archived]:
            note.parent.mkdir(parents=True)
            note.write_text(content)
        unrelated = self.notes / "unrelated.md"
        unrelated.write_text(f'---\nworktrees: []\n---\nExample from another task:\n    path: "{repo}"\n')
        result = self.run_script("new-task-note.sh", cwd=repo)
        self.assertEqual(Path(result.stdout.strip()), active)
        duplicate = self.notes / "legacy.md"
        duplicate.write_text(content)
        result = self.run_script("new-task-note.sh", cwd=repo, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Multiple notes", result.stderr)
        duplicate.unlink()
        active.unlink()
        result = self.run_script("new-task-note.sh", cwd=repo)
        self.assertEqual(Path(result.stdout.strip()), archived)

    def test_creation_requires_identity_and_rejects_traversal(self):
        for extra in [[], ["--task-id", "../escape"], ["--task-id", "_archive"]]:
            result = self.run_script("new-task-note.sh", "--branch", "feature", *extra, check=False)
            self.assertNotEqual(result.returncode, 0)
        for name in ["../escape.md", "task/../../escape.md", "/outside.md"]:
            result = self.run_script("archive-task-note.sh", name, check=False)
            self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
