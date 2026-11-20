import { useId, useState } from "react";
import {
  CONTRAST_LIMITS,
  FONT_AREAS,
  FONT_SIZE_LIMITS,
  FontArea,
  Typography,
} from "../../appearance";

const AREAS: Record<FontArea, { label: string; description: string; defaultFamily: string }> = {
  ui: {
    label: "Interface",
    description: "Everything outside chats, code and terminals. Zoom scales this size.",
    defaultFamily: "System font",
  },
  chat: {
    label: "Chat and prompts",
    description: "Chat messages and the prompt box.",
    defaultFamily: "Interface font",
  },
  code: {
    label: "Code",
    description: "Code blocks, diffs and file previews.",
    defaultFamily: "Menlo",
  },
  terminal: {
    label: "Terminal",
    description: "Agent and script terminals.",
    defaultFamily: "Code font",
  },
};

const CONTROL =
  "rounded-md border border-border bg-bg-tertiary px-2 py-1 text-sm text-text-primary outline-none focus:border-accent";

interface TypographySettingsProps {
  typography: Typography;
  onTypographyChange: (typography: Typography) => void;
  contrast: number;
  onContrastChange: (contrast: number) => void;
}

export function TypographySettings({
  typography,
  onTypographyChange,
  contrast,
  onContrastChange,
}: TypographySettingsProps) {
  const contrastId = useId();
  const change = (area: FontArea, update: Partial<Typography[FontArea]>) =>
    onTypographyChange({ ...typography, [area]: { ...typography[area], ...update } });

  return (
    <div className="px-6 pt-3 pb-6 space-y-5">
      {FONT_AREAS.map((area) => (
        <FontRow
          key={`${area}:${typography[area].family}`}
          area={area}
          family={typography[area].family}
          size={typography[area].size}
          onFamilyChange={(family) => change(area, { family })}
          onSizeChange={(size) => change(area, { size })}
        />
      ))}
      <div className="space-y-1">
        <label htmlFor={contrastId} className="flex items-center gap-2 text-sm text-text-primary">
          Contrast
          <span className="ml-auto text-xs text-text-muted tabular-nums">{contrast}%</span>
        </label>
        <input
          id={contrastId}
          type="range"
          min={CONTRAST_LIMITS.min}
          max={CONTRAST_LIMITS.max}
          step={5}
          value={contrast}
          onChange={(event) => onContrastChange(Number(event.target.value))}
          className="w-full accent-accent cursor-pointer"
        />
        <p className="text-xs text-text-muted">
          Moves secondary text and borders toward the main text color, or toward the background
          below 100%.
        </p>
      </div>
    </div>
  );
}

function FontRow({
  area,
  family,
  size,
  onFamilyChange,
  onSizeChange,
}: {
  area: FontArea;
  family: string;
  size: number;
  onFamilyChange: (family: string) => void;
  onSizeChange: (size: number) => void;
}) {
  const { label, description, defaultFamily } = AREAS[area];
  const { min, max } = FONT_SIZE_LIMITS[area];
  const [draft, setDraft] = useState(family);
  const commit = () => {
    if (draft.trim() !== family) onFamilyChange(draft.trim());
  };

  return (
    <div className="space-y-1">
      <p className="text-sm text-text-primary">{label}</p>
      <div className="flex gap-2">
        <input
          aria-label={`${label} font family`}
          value={draft}
          placeholder={defaultFamily}
          spellCheck={false}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") commit();
          }}
          className={`${CONTROL} flex-1 min-w-0`}
        />
        <select
          aria-label={`${label} font size`}
          value={size}
          onChange={(event) => onSizeChange(Number(event.target.value))}
          className={`${CONTROL} cursor-pointer`}
        >
          {Array.from({ length: max - min + 1 }, (_, index) => min + index).map((option) => (
            <option key={option} value={option}>
              {option}px
            </option>
          ))}
        </select>
      </div>
      <p className="text-xs text-text-muted">{description}</p>
    </div>
  );
}
