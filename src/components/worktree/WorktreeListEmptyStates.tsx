import { Button } from "../ui/Button";
import { FolderIcon, CodeBranchIcon } from "../ui/Icons";

export function WorktreeNoRepoPlaceholder() {
  return (
    <div className="flex-1 flex items-center justify-center" data-tauri-drag-region>
      <div className="text-center">
        <div className="w-12 h-12 rounded-xl bg-bg-tertiary flex items-center justify-center mx-auto mb-3">
          <FolderIcon className="text-text-muted" />
        </div>
        <p className="text-sm text-text-muted">Select a workspace or add one to get started</p>
      </div>
    </div>
  );
}

interface WorktreeEmptyWorktreesProps {
  onCreateFirst: () => void;
}

export function WorktreeEmptyWorktrees({ onCreateFirst }: WorktreeEmptyWorktreesProps) {
  return (
    <div className="flex items-center justify-center h-full">
      <div className="text-center">
        <div className="w-12 h-12 rounded-xl bg-bg-tertiary flex items-center justify-center mx-auto mb-3">
          <CodeBranchIcon className="text-text-muted" />
        </div>
        <p className="text-sm text-text-muted mb-3">No active tasks</p>
        <Button onClick={onCreateFirst} className="text-xs">
          Create your first task
        </Button>
      </div>
    </div>
  );
}
