import { useState } from "react";
import { generateThemeColors } from "../../themePalette";
import { Button } from "./Button";

interface ThemeGeneratorProps {
  colors: Record<string, string>;
  onGenerate: (colors: Record<string, string>) => void;
}

function ColorField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <label className="flex items-center gap-2 text-xs text-text-secondary">
      <input
        type="color"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="w-8 h-8 rounded-md border border-border bg-transparent cursor-pointer flex-shrink-0 p-0"
      />
      {label}
    </label>
  );
}

export function ThemeGenerator({ colors, onGenerate }: ThemeGeneratorProps) {
  const [background, setBackground] = useState(colors["bg-primary"] ?? "#0a0a0f");
  const [accent, setAccent] = useState(colors.accent ?? "#6366f1");

  return (
    <div className="flex flex-wrap items-center gap-4">
      <ColorField label="Background color" value={background} onChange={setBackground} />
      <ColorField label="Accent color" value={accent} onChange={setAccent} />
      <Button
        variant="secondary"
        className="ml-auto px-3 py-1 text-xs"
        onClick={() => onGenerate(generateThemeColors(background, accent))}
      >
        Generate theme
      </Button>
    </div>
  );
}
