import { useState, useEffect } from "react";
import { homeDir } from "@tauri-apps/api/path";
import { Modal } from "../ui/Modal";
import { Input } from "../ui/Input";
import { Button } from "../ui/Button";
import { Workspace, WorkspaceRepo } from "../../types";
import { WorkspaceRepoEditor } from "./WorkspaceRepoEditor";
import { LinearKeyField } from "./LinearKeyField";
import { useLinearKeyValidation } from "../../hooks/useLinearKeyValidation";

interface EditWorkspaceModalProps {
  open: boolean;
  onClose: () => void;
  workspace: Workspace;
  onSave: (
    workspaceId: string,
    updates: Partial<Pick<Workspace, "name" | "linearApiKey" | "repos">>,
  ) => void;
}

export function EditWorkspaceModal({ open, onClose, workspace, onSave }: EditWorkspaceModalProps) {
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
    setName(workspace.name);
    setRepos(workspace.repos);
    if (workspace.linearApiKey) {
      linear.setLinearKey(workspace.linearApiKey);
      linear.runValidation(workspace.linearApiKey);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const handleSave = () => {
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
    const newLinearKey = linear.linearValid ? linear.linearKey.trim() : null;
    onSave(workspace.id, {
      name: name.trim(),
      repos: repos.map((r) => ({
        ...r,
        name: r.name.trim(),
        worktreeBasePath: r.worktreeBasePath.trim(),
      })),
      linearApiKey: newLinearKey,
    });
    onClose();
  };

  return (
    <Modal open={open} onClose={onClose} title={`Edit ${workspace.name}`}>
      <div className="p-6 space-y-4">
        <Input label="Workspace name" value={name} onChange={(e) => setName(e.target.value)} />

        <WorkspaceRepoEditor repos={repos} onChange={setRepos} home={home} />

        <LinearKeyField
          label="Linear API key"
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
          <Button onClick={handleSave}>Save</Button>
        </div>
      </div>
    </Modal>
  );
}
