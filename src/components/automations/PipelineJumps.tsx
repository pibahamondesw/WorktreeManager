import { EdgeKind, PipelineEdge, transitionKey } from "../../services/automationPipeline";
import { PipelineStep } from "../../services/automations";

export const NODE_HEIGHT = 56;
export const NODE_WIDTH = 288;
export const LANE_WIDTH = 14;
const ROW_PITCH = NODE_HEIGHT + 24;
const ARROW_OFFSET = 6;

const EDGE_CLASSES: Record<EdgeKind, string> = {
  next: "text-text-muted",
  on_fail: "text-danger",
  on_reject: "text-warning",
};

const EDGE_LABELS: Record<EdgeKind, string> = {
  next: "next",
  on_fail: "on fail",
  on_reject: "on reject",
};

interface PipelineJumpsProps {
  edges: PipelineEdge[];
  steps: PipelineStep[];
  lanes: number;
  traversed: Map<string, number> | null;
}

const jumpTimes = (
  edge: PipelineEdge,
  steps: PipelineStep[],
  traversed: Map<string, number> | null
) => traversed?.get(transitionKey({ from: steps[edge.from].id, kind: edge.kind })) ?? 0;

const jumpLabel = (edge: PipelineEdge, steps: PipelineStep[]) =>
  `${steps[edge.from].id} → ${steps[edge.to].id} ${EDGE_LABELS[edge.kind]}${
    edge.limit ? ` (${edge.limit})` : ""
  }`;

export function JumpList({ edges, steps, traversed }: Omit<PipelineJumpsProps, "lanes">) {
  if (edges.length === 0) return null;
  return (
    <ul aria-label="Jumps" className="space-y-0.5 text-xs text-text-muted select-text">
      {edges.map((edge) => {
        const times = jumpTimes(edge, steps, traversed);
        return (
          <li key={`${edge.from}-${edge.kind}`} className="flex items-baseline gap-2">
            <span className={`font-mono ${times > 0 ? "text-text-primary" : ""}`}>
              {steps[edge.from].id} → {steps[edge.to].id}
            </span>
            <span className={EDGE_CLASSES[edge.kind]}>{EDGE_LABELS[edge.kind]}</span>
            {edge.limit && <span>{edge.limit}</span>}
            {traversed && <span>{times > 0 ? `taken ×${times}` : "not taken"}</span>}
          </li>
        );
      })}
    </ul>
  );
}

export function PipelineJumps({ edges, steps, lanes, traversed }: PipelineJumpsProps) {
  if (edges.length === 0) return null;
  const center = (index: number) => index * ROW_PITCH + NODE_HEIGHT / 2;
  return (
    <svg
      aria-hidden="true"
      className="absolute top-0 pointer-events-none overflow-visible"
      style={{ left: NODE_WIDTH }}
      width={lanes * LANE_WIDTH + 12}
      height={steps.length * ROW_PITCH - (ROW_PITCH - NODE_HEIGHT)}
    >
      {edges.map((edge) => {
        const x = (edge.lane + 1) * LANE_WIDTH;
        const start = center(edge.from) - ARROW_OFFSET;
        const end = center(edge.to) + ARROW_OFFSET;
        const label = jumpLabel(edge, steps);
        const times = jumpTimes(edge, steps, traversed);
        return (
          <g
            key={`${edge.from}-${edge.kind}`}
            data-edge={label}
            data-taken={traversed ? times : undefined}
            className={`${EDGE_CLASSES[edge.kind]} ${traversed && times === 0 ? "opacity-40" : ""}`}
            fill="none"
            stroke="currentColor"
            strokeWidth={times > 0 ? 2 : 1.25}
          >
            <title>{label}</title>
            <path
              d={`M0 ${start} H${x} V${end} H2`}
              strokeDasharray={edge.kind === "next" ? undefined : "4 3"}
            />
            <path d={`M7 ${end - 4} L2 ${end} L7 ${end + 4}`} />
          </g>
        );
      })}
    </svg>
  );
}
