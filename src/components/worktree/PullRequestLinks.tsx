import { prKey, useGithubPrStatuses } from "../../services/github";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Badge } from "../ui/Badge";
import { PullRequestInfo } from "../../types";
import {
  PullRequestClosedIcon,
  PullRequestConflictIcon,
  PullRequestDraftIcon,
  PullRequestIcon,
  PullRequestMergedIcon,
} from "../ui/Icons";
import { PullRequestChecksPopover } from "./PullRequestChecksPopover";

const prBadgeVariants: Record<string, "success" | "accent" | "danger" | "default"> = {
  merged: "accent",
  closed: "danger",
};

const isInactivePr = (state: string) => state === "draft" || state === "closed";

type PrDisplayState = "open" | "draft" | "merged" | "closed" | "conflicts";

const prStateIcons: Record<PrDisplayState, { Icon: typeof PullRequestIcon; className: string }> = {
  open: { Icon: PullRequestIcon, className: "text-success" },
  draft: { Icon: PullRequestDraftIcon, className: "text-text-muted" },
  merged: { Icon: PullRequestMergedIcon, className: "text-accent" },
  closed: { Icon: PullRequestClosedIcon, className: "text-danger" },
  conflicts: { Icon: PullRequestConflictIcon, className: "text-warning" },
};

function prDisplayState(state: string, draft: boolean, hasConflicts: boolean): PrDisplayState {
  if (state === "merged" || state === "closed") return state;
  if (hasConflicts) return "conflicts";
  return draft ? "draft" : "open";
}

interface PullRequestLinksProps {
  prs: PullRequestInfo[];
  onOpenError?: (msg: string) => void;
  onReady?: (pr: PullRequestInfo) => Promise<void>;
  numbersOnly?: boolean;
}

export function PullRequestLinks({
  prs,
  onOpenError,
  onReady,
  numbersOnly,
}: PullRequestLinksProps) {
  const statuses = useGithubPrStatuses();
  return prs.map((pr, index) => {
    const entry = statuses[prKey(pr)];
    const status = entry?.status;
    const state = status?.state ?? pr.state;
    const draft = status ? status.isDraft : pr.state === "draft";
    const showBadges = !numbersOnly;
    const displayState = prDisplayState(state, draft, !!status?.hasConflicts);
    const { Icon: StateIcon, className: stateIconClass } = prStateIcons[displayState];
    return (
      <span
        key={pr.url}
        className={`inline-flex items-center gap-1 ${numbersOnly ? "flex-shrink-0 whitespace-nowrap" : "flex-wrap"}`}
      >
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            openUrl(pr.url).catch(() => onOpenError?.("Could not open the pull request"));
          }}
          title={
            displayState === "open"
              ? `#${pr.number}: ${pr.title}`
              : `#${pr.number} (${displayState}): ${pr.title}`
          }
          className={`pointer-events-auto inline-flex items-center gap-1.5 transition-colors cursor-pointer ${
            isInactivePr(draft ? "draft" : state) ? "text-text-muted" : "text-accent"
          } hover:text-accent-hover`}
        >
          <StateIcon size={12} className={`flex-shrink-0 ${stateIconClass}`} />#{pr.number}
          {showBadges && state !== "open" && (
            <Badge variant={prBadgeVariants[state] ?? "default"}>{state}</Badge>
          )}
        </button>
        {showBadges && draft && state !== "draft" && <Badge>draft</Badge>}
        {showBadges && status && state === "open" && (
          <>
            {status.ci !== "none" && (
              <PullRequestChecksPopover pr={pr} onOpenError={onOpenError}>
                <Badge
                  variant={
                    status.ci === "passing"
                      ? "success"
                      : status.ci === "failing"
                        ? "danger"
                        : "default"
                  }
                >{`CI ${status.ci}`}</Badge>
              </PullRequestChecksPopover>
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
        {index < prs.length - 1 && <span className="-ml-1 text-text-muted">,</span>}
      </span>
    );
  });
}
