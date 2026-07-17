import { themes, THEME_COLOR_TOKENS } from "../../themes";

interface CustomThemeEditorProps {
  colors: Record<string, string>;
  onChange: (colors: Record<string, string>) => void;
}

// Distinct groups in the order tokens first appear.
const GROUPS = [...new Set(THEME_COLOR_TOKENS.map((t) => t.group))];

export function CustomThemeEditor({ colors, onChange }: CustomThemeEditorProps) {
  const setToken = (key: string, value: string) => {
    onChange({ ...colors, [key]: value });
  };

  const copyFromPreset = (id: string) => {
    const preset = themes.find((t) => t.id === id);
    if (preset) onChange({ ...preset.colors });
  };

  return (
    <div className="mt-4 border-t border-border pt-4">
      <div className="flex items-center justify-between mb-3">
        <p className="text-xs font-medium uppercase tracking-wide text-text-muted">
          Customize colors
        </p>
        <label className="flex items-center gap-2 text-xs text-text-secondary">
          Copy from
          <select
            value=""
            onChange={(e) => copyFromPreset(e.target.value)}
            className="bg-bg-tertiary border border-border rounded-md px-2 py-1 text-text-primary cursor-pointer focus:border-accent focus:outline-none"
          >
            <option value="" disabled>
              preset…
            </option>
            {themes.map((t) => (
              <option key={t.id} value={t.id}>
                {t.name}
              </option>
            ))}
          </select>
        </label>
      </div>

      <div className="space-y-4">
        {GROUPS.map((group) => (
          <div key={group}>
            <p className="text-xs font-semibold text-text-secondary mb-2">{group}</p>
            <div className="space-y-1.5">
              {THEME_COLOR_TOKENS.filter((tok) => tok.group === group).map((tok) => {
                const value = colors[tok.key] ?? "#000000";
                return (
                  <div key={tok.key} className="flex items-center gap-3">
                    <input
                      type="color"
                      value={value}
                      onChange={(e) => setToken(tok.key, e.target.value)}
                      className="w-8 h-8 rounded-md border border-border bg-transparent cursor-pointer flex-shrink-0 p-0"
                      aria-label={tok.label}
                    />
                    <span className="flex-1 min-w-0 truncate text-sm text-text-primary">
                      {tok.label}
                    </span>
                    <input
                      type="text"
                      value={value}
                      onChange={(e) => setToken(tok.key, e.target.value)}
                      spellCheck={false}
                      className="w-24 rounded-md border border-border bg-bg-tertiary px-2 py-1 font-mono text-sm text-text-secondary focus:border-accent focus:outline-none"
                    />
                  </div>
                );
              })}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
