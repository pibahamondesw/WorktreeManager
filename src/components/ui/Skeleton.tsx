const SHAPES = {
  line: "rounded",
  pill: "rounded-full",
};

export function Skeleton({
  className = "",
  shape = "line",
}: {
  className?: string;
  shape?: keyof typeof SHAPES;
}) {
  return (
    <div
      aria-hidden="true"
      className={`bg-bg-tertiary animate-pulse ${SHAPES[shape]} ${className}`}
    />
  );
}
