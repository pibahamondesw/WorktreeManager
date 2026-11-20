import { Skeleton } from "../ui/Skeleton";

export function WorktreeCardSkeleton({ index }: { index: number }) {
  return (
    <div className="bg-bg-secondary border border-border rounded-xl p-4">
      <div className="flex items-start justify-between gap-3">
        {index <= 9 && (
          <span className="w-4 pt-0.5 flex-shrink-0">
            <Skeleton className="h-3 w-3" />
          </span>
        )}
        <div className="min-w-0 flex-1">
          {/* Row 1: identifier + status */}
          <div className="flex items-center gap-2 mb-1">
            <Skeleton className="h-3.5 w-16" />
            <Skeleton shape="pill" className="h-4 w-20" />
            <Skeleton className="h-3 w-10 ml-auto" />
          </div>
          {/* Row 2: title */}
          <Skeleton className="h-4 w-3/4" />
          {/* Row 3: branch + git status */}
          <div className="flex items-center gap-3 mt-2">
            <Skeleton className="h-3 w-40" />
            <Skeleton className="h-3 w-8" />
          </div>
          {/* Row 4: PR link */}
          <div className="flex items-center gap-1.5 mt-2">
            <Skeleton className="h-3 w-3" />
            <Skeleton className="h-3 w-36" />
          </div>
        </div>
      </div>
    </div>
  );
}
