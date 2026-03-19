export function WorktreeCardSkeleton({ index }: { index: number }) {
  return (
    <div className="bg-bg-secondary border border-border rounded-xl p-4 animate-pulse">
      <div className="flex items-start justify-between gap-3">
        {index <= 9 && (
          <span className="w-4 pt-0.5 flex-shrink-0">
            <div className="h-3 w-3 rounded bg-bg-tertiary" />
          </span>
        )}
        <div className="min-w-0 flex-1">
          {/* Row 1: identifier + status */}
          <div className="flex items-center gap-2 mb-1">
            <div className="h-3.5 w-16 rounded bg-bg-tertiary" />
            <div className="h-4 w-20 rounded-full bg-bg-tertiary" />
            <div className="h-3 w-10 rounded bg-bg-tertiary ml-auto" />
          </div>
          {/* Row 2: title */}
          <div className="h-4 w-3/4 rounded bg-bg-tertiary" />
          {/* Row 3: branch + git status */}
          <div className="flex items-center gap-3 mt-2">
            <div className="h-3 w-40 rounded bg-bg-tertiary" />
            <div className="h-3 w-8 rounded bg-bg-tertiary" />
          </div>
          {/* Row 4: PR link */}
          <div className="flex items-center gap-1.5 mt-2">
            <div className="h-3 w-3 rounded bg-bg-tertiary" />
            <div className="h-3 w-36 rounded bg-bg-tertiary" />
          </div>
        </div>
      </div>
    </div>
  );
}
