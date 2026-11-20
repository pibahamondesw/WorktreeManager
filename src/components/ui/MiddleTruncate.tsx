const TAIL_LENGTH = 12;
const MAX_LAST_SEGMENT_LENGTH = 24;

function tailStart(value: string): number {
  if (value.length <= TAIL_LENGTH) return value.length;
  const lastSeparator = value.lastIndexOf("/", value.length - 2);
  if (lastSeparator > 0 && value.length - lastSeparator <= MAX_LAST_SEGMENT_LENGTH) {
    return lastSeparator;
  }
  return value.length - TAIL_LENGTH;
}

/** Shows the start and end of a long path or branch; the full value stays selectable as a whole. */
export function MiddleTruncate({ value, className = "" }: { value: string; className?: string }) {
  const split = tailStart(value);
  return (
    <span
      title={value}
      className={`inline-flex min-w-0 max-w-full overflow-hidden select-all ${className}`}
    >
      <span className="min-w-0 overflow-hidden text-ellipsis whitespace-pre">
        {value.slice(0, split)}
      </span>
      {split < value.length && (
        <span className="shrink-0 whitespace-pre">{value.slice(split)}</span>
      )}
    </span>
  );
}
