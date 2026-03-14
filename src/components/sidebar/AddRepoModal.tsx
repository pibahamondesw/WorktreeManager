import { useState, useEffect } from "react";
import { v4 as uuid } from "uuid";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { homeDir } from "@tauri-apps/api/path";
import { Modal } from "../ui/Modal";
import { Input } from "../ui/Input";
import { Button } from "../ui/Button";
import { Repo } from "../../types";

interface AddRepoModalProps {
  open: boolean;
  onClose: () => void;
  onAdd: (repo: Repo) => void;
}

export function AddRepoModal({ open, onClose, onAdd }: AddRepoModalProps) {
  const [name, setName] = useState("");
  const [localPath, setLocalPath] = useState("");
  const [worktreeBase, setWorktreeBase] = useState("");
  const [home, setHome] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    homeDir().then(setHome);
  }, []);

  const computeDefaultWorktreeBase = (projectName: string) => {
    if (!home || !projectName.trim()) return "";
    const slug = projectName
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/(^-|-$)/g, "");
    const base = home.endsWith("/") ? home : `${home}/`;
    return `${base}Documents/.worktreemanager/worktrees/${slug}`;
  };

  const handleBrowse = async () => {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: "Select repo directory",
    });
    if (selected) {
      setLocalPath(selected as string);
      if (!name) {
        const parts = (selected as string).split("/");
        const folderName = parts[parts.length - 1] || parts[parts.length - 2];
        if (folderName) {
          setName(folderName);
          setWorktreeBase(computeDefaultWorktreeBase(folderName));
        }
      }
    }
  };

  const handleNameChange = (newName: string) => {
    setName(newName);
    // Only auto-update worktreeBase if user hasn't manually edited it
    const currentDefault = computeDefaultWorktreeBase(name);
    if (!worktreeBase || worktreeBase === currentDefault) {
      setWorktreeBase(computeDefaultWorktreeBase(newName));
    }
  };

  const handleSubmit = () => {
    setError(null);
    if (!name.trim()) {
      setError("Name is required");
      return;
    }
    if (!localPath.trim()) {
      setError("Local path is required. Use Browse to select your cloned repo.");
      return;
    }
    if (!localPath.startsWith("/")) {
      setError("Local path must be an absolute path (starting with /)");
      return;
    }
    const finalWorktreeBase = worktreeBase.trim() || computeDefaultWorktreeBase(name);
    if (!finalWorktreeBase) {
      setError("Worktree base path is required");
      return;
    }

    onAdd({
      id: uuid(),
      name: name.trim(),
      localPath: localPath.trim(),
      worktreeBasePath: finalWorktreeBase,
    });

    setName("");
    setLocalPath("");
    setWorktreeBase("");
    setError(null);
  };

  return (
    <Modal open={open} onClose={onClose} title="Add Project">
      <div className="p-6 space-y-4">
        <Input
          label="Project name"
          placeholder="e.g. Fintoc Rails"
          value={name}
          onChange={(e) => handleNameChange(e.target.value)}
        />

        <div className="flex flex-col gap-1.5">
          <label className="text-sm font-medium text-text-secondary">Local clone path</label>
          <div className="flex gap-2">
            <input
              className="flex-1 rounded-lg border border-border bg-bg-tertiary px-3 py-2 text-sm text-text-primary placeholder:text-text-muted outline-none transition-colors focus:border-accent"
              placeholder="/Users/.../my-repo"
              value={localPath}
              onChange={(e) => setLocalPath(e.target.value)}
              readOnly
            />
            <Button variant="secondary" onClick={handleBrowse}>
              Browse
            </Button>
          </div>
          <p className="text-xs text-text-muted">The git repo you've already cloned locally</p>
        </div>

        <Input
          label="Worktree directory"
          placeholder="Auto-filled from project name"
          value={worktreeBase}
          onChange={(e) => setWorktreeBase(e.target.value)}
          hint="Where new worktrees will be created (each branch gets a subfolder)"
        />

        {error && <p className="text-sm text-danger select-text">{error}</p>}

        <div className="flex justify-end gap-3 pt-2">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={handleSubmit}>Add Project</Button>
        </div>
      </div>
    </Modal>
  );
}
