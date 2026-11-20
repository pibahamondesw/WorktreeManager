import { ReactNode, useEffect, useRef, useState } from "react";

const FEEDBACK = {
  copied: { message: "Copied", className: "text-text-primary", durationMs: 1500 },
  failed: { message: "Could not copy", className: "text-danger", durationMs: 5000 },
};

type CopyOutcome = keyof typeof FEEDBACK;

/** Copies `value` and confirms the outcome in a bubble anchored above the button. */
export function CopyButton({
  value,
  className = "",
  title,
  children,
}: {
  value: string;
  className?: string;
  title?: string;
  children: ReactNode;
}) {
  const [outcome, setOutcome] = useState<CopyOutcome | null>(null);
  const resetTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(resetTimer.current), []);

  const show = (next: CopyOutcome) => {
    clearTimeout(resetTimer.current);
    setOutcome(next);
    resetTimer.current = setTimeout(() => setOutcome(null), FEEDBACK[next].durationMs);
  };

  const copy = () =>
    navigator.clipboard.writeText(value).then(
      () => show("copied"),
      () => show("failed")
    );

  return (
    <span className="relative inline-flex min-w-0">
      <button type="button" onClick={() => void copy()} className={className} title={title}>
        {children}
      </button>
      <span role="status" className="absolute bottom-full left-1/2 mb-1 -translate-x-1/2">
        {outcome && (
          <span
            className={`block whitespace-nowrap rounded-md border border-border bg-bg-active px-1.5 py-0.5 text-[0.625rem] shadow-lg motion-fade select-text ${FEEDBACK[outcome].className}`}
          >
            {FEEDBACK[outcome].message}
          </span>
        )}
      </span>
    </span>
  );
}
