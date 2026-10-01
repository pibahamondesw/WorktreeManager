import { openUrl } from "@tauri-apps/plugin-opener";
import { Badge } from "../ui/Badge";
import { PullRequestInfo } from "../../types";

const prBadgeVariants: Record<string, "success" | "accent" | "danger" | "default"> = {
  open: "success",
  merged: "accent",
  closed: "danger",
};

const isInactivePr = (state: string) => state === "draft" || state === "closed";

interface PullRequestLinksProps {
  prs: PullRequestInfo[];
  onOpenError?: (msg: string) => void;
}

export function PullRequestLinks({ prs, onOpenError }: PullRequestLinksProps) {
  return prs.map((pr, index) => (
    <span key={pr.url} className="inline-flex items-center">
      <button
        type="button"
        onClick={(event) => {
          event.stopPropagation();
          openUrl(pr.url).catch(() => onOpenError?.("Could not open the pull request"));
        }}
        title={
          pr.state === "draft" ? `#${pr.number} (draft): ${pr.title}` : `#${pr.number}: ${pr.title}`
        }
        className={`pointer-events-auto inline-flex items-center gap-1.5 transition-colors cursor-pointer ${
          isInactivePr(pr.state) ? "text-text-muted" : "text-accent"
        } hover:text-accent-hover`}
      >
        #{pr.number}
        <Badge variant={prBadgeVariants[pr.state] ?? "default"}>{pr.state}</Badge>
      </button>
      {index < prs.length - 1 && <span className="text-text-muted">,</span>}
    </span>
  ));
}
