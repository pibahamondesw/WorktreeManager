import { useState, useEffect, useMemo, useRef, ReactNode, KeyboardEvent } from "react";
import { LinearIssue } from "../../types";
import { useLinear } from "../../contexts/useLinear";
import { useDebounce } from "../../hooks/useDebounce";
import { Badge } from "../ui/Badge";
import { SpinnerIcon, SearchIcon } from "../ui/Icons";

const priorityLabels: Record<number, string> = {
  0: "No priority",
  1: "Urgent",
  2: "High",
  3: "Medium",
  4: "Low",
};

const priorityVariants: Record<number, "danger" | "warning" | "accent" | "default"> = {
  1: "danger",
  2: "warning",
  3: "accent",
  4: "default",
};

export function LinearIssuePicker({
  onSelect,
  children,
}: {
  onSelect: (issue: LinearIssue) => void;
  children?: ReactNode;
}) {
  const linear = useLinear();
  const [query, setQuery] = useState("");
  const [cachedIssues, setCachedIssues] = useState<LinearIssue[]>([]);
  const [remoteResults, setRemoteResults] = useState<LinearIssue[]>([]);
  const [loading, setLoading] = useState(true);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeIndex, setActiveIndex] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const debouncedQuery = useDebounce(query, 300);

  useEffect(() => {
    let active = true;
    setCachedIssues([]);
    setLoading(true);
    setError(null);
    if (!linear) {
      setLoading(false);
      return;
    }
    linear
      .fetchAssignedIssues()
      .then((issues) => {
        if (active) setCachedIssues(issues);
      })
      .catch(() => {
        if (active) setError("Failed to load issues. Try again by reopening this dialog.");
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [linear]);

  useEffect(() => {
    let active = true;
    setRemoteResults([]);
    setSearching(false);
    if (!linear || !query.trim() || query !== debouncedQuery) return;
    setSearching(true);
    setError(null);
    linear
      .fetchAssignedIssues(debouncedQuery)
      .then((issues) => {
        if (active) setRemoteResults(issues);
      })
      .catch(() => {
        if (active) setError("Failed to search issues. Try another search.");
      })
      .finally(() => {
        if (active) setSearching(false);
      });
    return () => {
      active = false;
    };
  }, [query, debouncedQuery, linear]);

  const issues = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return cachedIssues;
    const local = cachedIssues.filter((issue) =>
      [issue.identifier, issue.title, issue.projectName, issue.description, issue.branchName].some(
        (value) => value?.toLowerCase().includes(q)
      )
    );
    const localIds = new Set(local.map((issue) => issue.id));
    return [...local, ...remoteResults.filter((issue) => !localIds.has(issue.id))];
  }, [query, cachedIssues, remoteResults]);

  useEffect(() => setActiveIndex(0), [query]);

  useEffect(() => {
    setActiveIndex((i) => Math.min(i, Math.max(issues.length - 1, 0)));
  }, [issues.length]);

  useEffect(() => {
    listRef.current?.querySelector('[data-active="true"]')?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, issues]);

  const resultsVisible = !children && !loading;

  const handleKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (!resultsVisible || issues.length === 0) return;
    if (e.key === "ArrowDown" || (e.key === "n" && e.ctrlKey)) {
      e.preventDefault();
      setActiveIndex((i) => Math.min(i + 1, issues.length - 1));
      return;
    }
    if (e.key === "ArrowUp" || (e.key === "p" && e.ctrlKey)) {
      e.preventDefault();
      setActiveIndex((i) => Math.max(i - 1, 0));
      return;
    }
    if (/^[0-9]$/.test(e.key) && !e.metaKey && !e.ctrlKey && !e.altKey && !query.trim()) {
      const index = Number(e.key);
      if (index < issues.length) {
        e.preventDefault();
        setActiveIndex(index);
      }
      return;
    }
    if (e.key === "Enter" && issues[activeIndex]) {
      e.preventDefault();
      onSelect(issues[activeIndex]);
    }
  };

  return (
    <>
      <div className="px-6 py-3 border-b border-border">
        <div className="relative">
          <SearchIcon className="absolute left-3 top-1/2 -translate-y-1/2 text-text-muted" />
          <input
            className="w-full pl-9 pr-3 py-2 rounded-lg border border-border bg-bg-tertiary text-sm text-text-primary placeholder:text-text-muted outline-none focus:border-accent transition-colors"
            aria-label="Search Linear issues"
            placeholder="Search by issue ID, title, project, or description..."
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={handleKeyDown}
            autoFocus
          />
        </div>
      </div>

      <div ref={listRef} className="flex-1 overflow-y-auto">
        {children ?? (
          <>
            {loading && (
              <div className="flex items-center justify-center py-12">
                <SpinnerIcon size={20} className="text-text-muted" />
              </div>
            )}

            {!loading && issues.length === 0 && !searching && (
              <div className="flex items-center justify-center py-12">
                <p className="text-sm text-text-muted">
                  {query ? "No issues found" : "No assigned issues in progress"}
                </p>
              </div>
            )}

            {!loading && issues.length === 0 && searching && (
              <div className="flex items-center justify-center py-12">
                <p className="text-sm text-text-muted">Searching...</p>
              </div>
            )}

            {!loading &&
              issues.map((issue, index) => (
                <button
                  key={issue.id}
                  data-active={index === activeIndex ? "true" : undefined}
                  onMouseMove={() => setActiveIndex(index)}
                  onClick={() => onSelect(issue)}
                  className={`w-full flex items-center gap-3 px-6 py-3 transition-colors text-left cursor-pointer border-b border-border/50 last:border-0 ${
                    index === activeIndex ? "bg-bg-hover" : "hover:bg-bg-hover/50"
                  }`}
                >
                  {index <= 9 && (
                    <span className="text-xs font-mono text-text-muted/40 flex-shrink-0 w-4 text-right">
                      {index}
                    </span>
                  )}
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-xs font-mono text-text-muted flex-shrink-0">
                        {issue.identifier}
                      </span>
                      <span className="text-sm text-text-primary truncate">{issue.title}</span>
                    </div>
                    <div className="flex items-center gap-2 mt-1">
                      {issue.projectName && (
                        <span className="text-xs text-text-muted">{issue.projectName}</span>
                      )}
                    </div>
                  </div>
                  <div className="flex items-center gap-2 flex-shrink-0">
                    {issue.priority > 0 && (
                      <Badge variant={priorityVariants[issue.priority] ?? "default"}>
                        {priorityLabels[issue.priority] ?? ""}
                      </Badge>
                    )}
                    {issue.stateName && <Badge>{issue.stateName}</Badge>}
                  </div>
                </button>
              ))}

            {error && (
              <div className="px-6 py-3">
                <div className="rounded-lg bg-danger/10 border border-danger/20 px-3 py-2">
                  <p role="alert" className="text-sm text-danger select-text cursor-text">
                    {error}
                  </p>
                </div>
              </div>
            )}
          </>
        )}
      </div>
    </>
  );
}
