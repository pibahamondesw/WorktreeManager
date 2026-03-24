import { useState, useEffect, useRef } from "react";
import { Modal } from "../ui/Modal";
import { Input } from "../ui/Input";
import { Button } from "../ui/Button";
import { SuccessCircleIcon } from "../ui/Icons";
import { validateLinearToken } from "../../services/linear";
import { Repo } from "../../types";

interface EditRepoModalProps {
  open: boolean;
  onClose: () => void;
  repo: Repo;
  onSave: (repoId: string, updates: { name?: string; linearApiKey?: string | null }) => void;
}

export function EditRepoModal({ open, onClose, repo, onSave }: EditRepoModalProps) {
  const [name, setName] = useState("");
  const [linearKey, setLinearKey] = useState("");
  const [linearValid, setLinearValid] = useState(false);
  const [linearUser, setLinearUser] = useState<string | null>(null);
  const [linearValidating, setLinearValidating] = useState(false);
  const [linearError, setLinearError] = useState<string | null>(null);
  const validatingKeyRef = useRef("");

  useEffect(() => {
    if (!open) {
      setName("");
      setLinearKey("");
      setLinearValid(false);
      setLinearUser(null);
      setLinearValidating(false);
      setLinearError(null);
      validatingKeyRef.current = "";
      return;
    }
    setName(repo.name);
    if (repo.linearApiKey) {
      setLinearKey(repo.linearApiKey);
      runValidation(repo.linearApiKey);
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

  const newLinearKey = linearValid ? linearKey.trim() : null;
  const nameChanged = name.trim() !== repo.name;
  const keyChanged = newLinearKey !== (repo.linearApiKey ?? null);
  const hasChanges = (nameChanged && name.trim()) || keyChanged;

  const handleSave = () => {
    const updates: { name?: string; linearApiKey?: string | null } = {};
    if (nameChanged && name.trim()) updates.name = name.trim();
    if (keyChanged) updates.linearApiKey = newLinearKey;
    onSave(repo.id, updates);
    onClose();
  };

  return (
    <Modal open={open} onClose={onClose} title={`Edit ${repo.name}`}>
      <div className="p-6 space-y-4">
        <Input
          label="Project name"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />

        <div className="flex flex-col gap-1.5">
          <label className="text-sm font-medium text-text-secondary">Linear API key</label>
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
              autoFocus
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

        <div className="flex justify-end gap-3 pt-2">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={handleSave} disabled={!hasChanges}>
            Save
          </Button>
        </div>
      </div>
    </Modal>
  );
}
