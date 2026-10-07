import { prKey, useGithubPrStatuses } from "../../services/github";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Badge } from "../ui/Badge";
import { PullRequestInfo } from "../../types";

const prBadgeVariants: Record<string, "success" | "accent" | "danger" | "default"> = {
  merged: "accent",
  closed: "danger",
};

const isInactivePr = (state: string) => state === "draft" || state === "closed";

interface PullRequestLinksProps {
  prs: PullRequestInfo[];
  onOpenError?: (msg: string) => void;
  onReady?: (pr: PullRequestInfo) => Promise<void>;
}

export function PullRequestLinks({ prs, onOpenError, onReady }: PullRequestLinksProps) {
  const statuses = useGithubPrStatuses();
  return prs.map((pr, index) => {
    const entry = statuses[prKey(pr)];
    const status = entry?.status;
    const state = status?.state ?? pr.state;
    const draft = status ? status.isDraft : pr.state === "draft";
    return (
      <span key={pr.url} className="inline-flex items-center gap-1 flex-wrap">
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            openUrl(pr.url).catch(() => onOpenError?.("Could not open the pull request"));
          }}
          title={draft ? `#${pr.number} (draft): ${pr.title}` : `#${pr.number}: ${pr.title}`}
          className={`pointer-events-auto inline-flex items-center gap-1.5 transition-colors cursor-pointer ${
            isInactivePr(draft ? "draft" : state) ? "text-text-muted" : "text-accent"
          } hover:text-accent-hover`}
        >
          #{pr.number}
          {state !== "open" && <Badge variant={prBadgeVariants[state] ?? "default"}>{state}</Badge>}
        </button>
        {draft && state !== "draft" && <Badge>draft</Badge>}
        {status && state === "open" && (
          <>
            {status.ci !== "none" && (
              <Badge
                variant={
                  status.ci === "passing"
                    ? "success"
                    : status.ci === "failing"
                      ? "danger"
                      : "default"
                }
              >{`CI ${status.ci}`}</Badge>
            )}
            <Badge
              variant={
                status.review === "approved"
                  ? "success"
                  : status.review === "changes_requested"
                    ? "danger"
                    : "default"
              }
            >
              {status.review === "approved"
                ? "Approved"
                : status.review === "changes_requested"
                  ? "Changes requested"
                  : "Review pending"}
            </Badge>
            {draft && onReady && (
              <button
                type="button"
                disabled={entry.pendingReady}
                className="pointer-events-auto text-accent hover:text-accent-hover disabled:opacity-50 cursor-pointer"
                onKeyDown={(event) => {
                  if (event.key === "Enter" || event.key === " ") event.stopPropagation();
                }}
                onClick={(event) => {
                  event.stopPropagation();
                  void onReady(pr).catch((error) => onOpenError?.(String(error)));
                }}
              >
                {entry.pendingReady ? "Marking ready…" : "Mark as ready"}
              </button>
            )}
          </>
        )}
        {index < prs.length - 1 && <span className="text-text-muted">,</span>}
      </span>
    );
  });
}
