import { openUrl } from "@tauri-apps/plugin-opener";
import { Badge } from "../ui/Badge";
import { PullRequestInfo } from "../../types";

const prBadgeVariant = (state: string) =>
  state === "open" ? "success" : state === "merged" ? "accent" : "default";

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
        title={`#${pr.number}: ${pr.title}`}
        className="pointer-events-auto inline-flex items-center gap-1.5 text-accent hover:text-accent-hover transition-colors cursor-pointer"
      >
        #{pr.number}
        <Badge variant={prBadgeVariant(pr.state)}>{pr.state}</Badge>
      </button>
      {index < prs.length - 1 && <span className="text-text-muted">,</span>}
    </span>
  ));
}
