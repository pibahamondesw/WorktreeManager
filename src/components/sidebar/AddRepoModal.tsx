import { useState, useEffect, useRef } from "react";
import { v4 as uuid } from "uuid";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { homeDir } from "@tauri-apps/api/path";
import { Modal } from "../ui/Modal";
import { Input } from "../ui/Input";
import { Button } from "../ui/Button";
import { SuccessCircleIcon } from "../ui/Icons";
import { validateLinearToken } from "../../services/linear";
import { Repo } from "../../types";

interface AddRepoModalProps {
  open: boolean;
  onClose: () => void;
  onAdd: (repo: Repo) => void;
  defaultLinearApiKey?: string | null;
}

export function AddRepoModal({ open, onClose, onAdd, defaultLinearApiKey }: AddRepoModalProps) {
  const [name, setName] = useState("");
  const [localPath, setLocalPath] = useState("");
  const [worktreeBase, setWorktreeBase] = useState("");
  const [home, setHome] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [linearKey, setLinearKey] = useState("");
  const [linearValid, setLinearValid] = useState(false);
  const [linearUser, setLinearUser] = useState<string | null>(null);
  const [linearValidating, setLinearValidating] = useState(false);
  const [linearError, setLinearError] = useState<string | null>(null);
  const validatingKeyRef = useRef("");

  useEffect(() => {
    homeDir().then(setHome);
  }, []);

  useEffect(() => {
    if (!open) {
      setName("");
      setLocalPath("");
      setWorktreeBase("");
      setError(null);
      setLinearKey("");
      setLinearValid(false);
      setLinearUser(null);
      setLinearValidating(false);
      setLinearError(null);
      validatingKeyRef.current = "";
      return;
    }
    if (defaultLinearApiKey) {
      setLinearKey(defaultLinearApiKey);
      runValidation(defaultLinearApiKey);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const runValidation = async (key: string) => {
    const trimmed = key.trim();
    if (!trimmed) return;
    validatingKeyRef.current = trimmed;
    setLinearValid(false);
    setLinearUser(null);
    setLinearError(null);
    setLinearValidating(true);
    try {
      const result = await validateLinearToken(trimmed);
      if (validatingKeyRef.current !== trimmed) return;
      setLinearValid(result.valid);
      setLinearUser(result.name ?? null);
      if (!result.valid) setLinearError(result.error ?? "Validation failed");
    } catch {
      if (validatingKeyRef.current !== trimmed) return;
      setLinearError("Validation failed");
    } finally {
      if (validatingKeyRef.current === trimmed) setLinearValidating(false);
    }
  };

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
      linearApiKey: linearValid ? linearKey.trim() : null,
    });
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

        {/* Linear API Key (optional) */}
        <div className="flex flex-col gap-1.5">
          <label className="text-sm font-medium text-text-secondary">Linear API key (optional)</label>
          <div className="flex gap-2">
            <input
              className={`flex-1 rounded-lg border bg-bg-tertiary px-3 py-2 text-sm text-text-primary placeholder:text-text-muted outline-none transition-colors focus:border-accent ${
                linearError ? "border-danger" : "border-border"
              }`}
              placeholder="lin_api_..."
              value={linearKey}
              onChange={(e) => {
                setLinearKey(e.target.value);
                setLinearValid(false);
                setLinearUser(null);
                setLinearError(null);
                validatingKeyRef.current = "";
              }}
              type="password"
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  runValidation(linearKey);
                }
              }}
            />
            <Button
              variant="secondary"
              onClick={() => runValidation(linearKey)}
              loading={linearValidating}
              disabled={!linearKey.trim() || linearValid}
            >
              {linearValid ? "Valid" : "Validate"}
            </Button>
          </div>
          {linearValid && linearUser && (
            <div className="flex items-center gap-1.5 text-xs text-success">
              <SuccessCircleIcon size={12} />
              Connected as {linearUser}
            </div>
          )}
          {linearError && <p className="text-xs text-danger">{linearError}</p>}
          <p className="text-xs text-text-muted">
            Connect Linear to search issues when creating worktrees
          </p>
        </div>

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
