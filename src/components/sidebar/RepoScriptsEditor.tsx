import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { RepoScripts, WorkspaceRepo } from "../../types";
import {
  approveScripts,
  resolveRepoScripts,
  ResolvedScripts,
  SCRIPT_SOURCE_LABELS,
} from "../../services/repoScripts";
import { Button } from "../ui/Button";

interface SetupSuggestion {
  setup: string;
  include: string[];
}

const textareaClass =
  "w-full rounded-md border border-border bg-bg-secondary px-2 py-1 text-xs text-text-primary outline-none focus:border-accent font-mono resize-y";

function errorText(error: unknown) {
  return typeof error === "string" ? error : error instanceof Error ? error.message : "Failed";
}

/**
 * Setup and teardown for one repository: what its checkout declares, an app-only override, and
 * a draft generated from what the app would otherwise detect, which can be kept as the override
 * or written to the repository to commit.
 */
export function RepoScriptsEditor({
  repo,
  onChange,
}: {
  repo: WorkspaceRepo;
  onChange: (scripts: RepoScripts | undefined) => void;
}) {
  const [declared, setDeclared] = useState<ResolvedScripts | null>(null);
  const [include, setInclude] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<{ error: boolean; text: string } | null>(null);
  const scripts = repo.scripts ?? {};

  const load = () => {
    if (declared) return;
    resolveRepoScripts(repo.localPath)
      .then(setDeclared)
      .catch((error) => setNotice({ error: true, text: errorText(error) }));
  };

  const setPhase = (phase: keyof RepoScripts, value: string) => {
    const next = { ...scripts, [phase]: value };
    onChange(next.setup?.trim() || next.teardown?.trim() ? next : undefined);
  };

  const draft = async () => {
    setBusy(true);
    setNotice(null);
    try {
      const suggestion = await invoke<SetupSuggestion>("suggest_repo_setup", {
        repoPath: repo.localPath,
      });
      onChange({ ...scripts, setup: suggestion.setup });
      setInclude(suggestion.include.join("\n"));
      setNotice({
        error: false,
        text: "Drafted from the repository. Review it, then save to keep it in the app or write it to the repository.",
      });
    } catch (error) {
      setNotice({ error: true, text: errorText(error) });
    } finally {
      setBusy(false);
    }
  };

  const writeToRepository = async () => {
    setBusy(true);
    setNotice(null);
    try {
      const written = await invoke<string[]>("write_repo_setup", {
        repoPath: repo.localPath,
        setup: scripts.setup ?? "",
        teardown: scripts.teardown ?? "",
        include: (include ?? "").split("\n"),
      });
      const resolved = await resolveRepoScripts(repo.localPath);
      const hashes = [resolved.setupHash, resolved.teardownHash].filter(
        (hash): hash is string => !!hash
      );
      if (hashes.length) await approveScripts(repo.localPath, hashes);
      setDeclared(resolved);
      onChange(undefined);
      setNotice({
        error: false,
        text: `Created ${written.join(" and ")} in ${repo.localPath}. Commit it to share; new tasks branch from the remote default branch, so they use it once it is pushed.`,
      });
    } catch (error) {
      setNotice({ error: true, text: errorText(error) });
    } finally {
      setBusy(false);
    }
  };

  const source = declared?.source;
  return (
    <details onToggle={(event) => event.currentTarget.open && load()}>
      <summary className="cursor-pointer text-xs text-text-secondary">
        Setup scripts
        {repo.scripts && <span className="text-text-muted"> · app override</span>}
      </summary>
      <div className="mt-2 space-y-2">
        <p className="text-xs text-text-muted select-text">
          {declared === null
            ? "Reading the repository…"
            : source
              ? `The repository declares scripts in ${SCRIPT_SOURCE_LABELS[source]}. A phase set here overrides it, only in this app.`
              : "The repository declares no scripts, so the app detects Doppler, Node and Python. Scripts set here replace that detection."}
        </p>
        {(["setup", "teardown"] as const).map((phase) => (
          <label key={phase} className="block space-y-1">
            <span className="text-xs text-text-secondary">
              {phase === "setup"
                ? "Setup, after creating the worktree"
                : "Teardown, before deleting it"}
            </span>
            <textarea
              className={textareaClass}
              rows={phase === "setup" ? 4 : 2}
              spellCheck={false}
              value={scripts[phase] ?? ""}
              placeholder={declared?.[phase] ?? undefined}
              onChange={(event) => setPhase(phase, event.target.value)}
            />
          </label>
        ))}
        {include !== null && (
          <label className="block space-y-1">
            <span className="text-xs text-text-secondary">
              .worktreeinclude, ignored files copied from the main checkout
            </span>
            <textarea
              className={textareaClass}
              rows={3}
              spellCheck={false}
              value={include}
              onChange={(event) => setInclude(event.target.value)}
            />
          </label>
        )}
        {notice && (
          <p className={`text-xs select-text ${notice.error ? "text-danger" : "text-text-muted"}`}>
            {notice.text}
          </p>
        )}
        <div className="flex gap-2">
          <Button
            variant="secondary"
            className="px-2 py-1 text-xs"
            loading={busy}
            onClick={() => void draft()}
          >
            Draft from repository
          </Button>
          <Button
            variant="secondary"
            className="px-2 py-1 text-xs"
            disabled={
              busy || !(scripts.setup?.trim() || scripts.teardown?.trim() || include?.trim())
            }
            onClick={() => void writeToRepository()}
            title={`Create .worktreemanager.toml${include?.trim() ? " and .worktreeinclude" : ""} in the main checkout`}
          >
            Write to repository
          </Button>
        </div>
      </div>
    </details>
  );
}
