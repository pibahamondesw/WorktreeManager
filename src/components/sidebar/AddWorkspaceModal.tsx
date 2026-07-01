import { useState, useEffect } from "react";
import { v4 as uuid } from "uuid";
import { homeDir } from "@tauri-apps/api/path";
import { Modal } from "../ui/Modal";
import { Input } from "../ui/Input";
import { Button } from "../ui/Button";
import { Workspace, WorkspaceRepo } from "../../types";
import { WorkspaceRepoEditor } from "./WorkspaceRepoEditor";
import { LinearKeyField } from "./LinearKeyField";
import { useLinearKeyValidation } from "../../hooks/useLinearKeyValidation";

interface AddWorkspaceModalProps {
  open: boolean;
  onClose: () => void;
  onAdd: (workspace: Workspace) => void;
  defaultLinearApiKey?: string | null;
}

export function AddWorkspaceModal({
  open,
  onClose,
  onAdd,
  defaultLinearApiKey,
}: AddWorkspaceModalProps) {
  const [name, setName] = useState("");
  const [repos, setRepos] = useState<WorkspaceRepo[]>([]);
  const [home, setHome] = useState("");
  const [error, setError] = useState<string | null>(null);
  const linear = useLinearKeyValidation();

  useEffect(() => {
    homeDir().then(setHome);
  }, []);

  useEffect(() => {
    if (!open) {
      setName("");
      setRepos([]);
      setError(null);
      linear.reset();
      return;
    }
    if (defaultLinearApiKey) {
      linear.setLinearKey(defaultLinearApiKey);
      linear.runValidation(defaultLinearApiKey);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const handleSubmit = () => {
    setError(null);
    if (!name.trim()) {
      setError("Workspace name is required");
      return;
    }
    if (repos.length === 0) {
      setError("Add at least one repository");
      return;
    }
    for (const r of repos) {
      if (!r.localPath.startsWith("/")) {
        setError(`${r.name || "A repo"}: local path must be absolute`);
        return;
      }
      if (!r.worktreeBasePath.trim()) {
        setError(`${r.name || "A repo"}: worktree base path is required`);
        return;
      }
    }
    onAdd({
      id: uuid(),
      name: name.trim(),
      repos: repos.map((r) => ({
        ...r,
        name: r.name.trim(),
        worktreeBasePath: r.worktreeBasePath.trim(),
      })),
      linearApiKey: linear.linearValid ? linear.linearKey.trim() : null,
    });
  };

  return (
    <Modal open={open} onClose={onClose} title="Add Workspace">
      <div className="p-6 space-y-4">
        <Input
          label="Workspace name"
          placeholder="e.g. Payments"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />

        <WorkspaceRepoEditor repos={repos} onChange={setRepos} home={home} />

        <LinearKeyField
          linearKey={linear.linearKey}
          linearValid={linear.linearValid}
          linearUser={linear.linearUser}
          linearValidating={linear.linearValidating}
          linearError={linear.linearError}
          onChange={linear.setLinearKey}
          onValidate={() => linear.runValidation(linear.linearKey)}
        />

        {error && <p className="text-sm text-danger select-text">{error}</p>}

        <div className="flex justify-end gap-3 pt-2">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={handleSubmit}>Add Workspace</Button>
        </div>
      </div>
    </Modal>
  );
}
