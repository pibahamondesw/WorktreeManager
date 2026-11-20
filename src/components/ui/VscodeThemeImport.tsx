import { ChangeEvent, FormEvent, useRef, useState } from "react";
import { loadOpenVsxThemes, OpenVsxExtension, searchOpenVsxThemes } from "../../services/openVsx";
import { ImportedTheme, themeFromFile } from "../../vscodeTheme";
import { themesFromVsix } from "../../vsixThemes";
import { Button } from "./Button";
import { EmptyState } from "./EmptyState";
import { Input } from "./Input";

interface VscodeThemeImportProps {
  onImport: (colors: Record<string, string>) => void;
}

interface ThemeChoice {
  source: string;
  themes: ImportedTheme[];
}

async function themesFromLocalFile(file: File): Promise<ImportedTheme[]> {
  if (file.name.toLowerCase().endsWith(".vsix")) {
    return themesFromVsix(new Uint8Array(await file.arrayBuffer()));
  }
  return [themeFromFile(await file.text(), file.name)];
}

const errorMessage = (error: unknown) => (error instanceof Error ? error.message : String(error));

export function VscodeThemeImport({ onImport }: VscodeThemeImportProps) {
  const fileInput = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState("");
  const [searching, setSearching] = useState(false);
  const [results, setResults] = useState<OpenVsxExtension[] | null>(null);
  const [loadingId, setLoadingId] = useState<string | null>(null);
  const [choice, setChoice] = useState<ThemeChoice | null>(null);
  const [error, setError] = useState<string | null>(null);

  const offerThemes = (source: string, themes: ImportedTheme[]) => {
    if (themes.length === 1) onImport(themes[0].colors);
    setChoice({ source, themes });
  };

  const run = async (task: () => Promise<void>) => {
    setError(null);
    try {
      await task();
    } catch (cause) {
      setError(errorMessage(cause));
    }
  };

  const importFile = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) return;
    void run(async () => offerThemes(file.name, await themesFromLocalFile(file)));
  };

  const search = (event: FormEvent) => {
    event.preventDefault();
    if (!query.trim()) return;
    setSearching(true);
    void run(async () => setResults(await searchOpenVsxThemes(query))).finally(() =>
      setSearching(false)
    );
  };

  const loadExtension = (extension: OpenVsxExtension) => {
    setLoadingId(extension.id);
    void run(async () => offerThemes(extension.name, await loadOpenVsxThemes(extension))).finally(
      () => setLoadingId(null)
    );
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <form onSubmit={search} className="flex flex-1 items-center gap-2">
          <div className="flex-1">
            <Input
              aria-label="Search Open VSX themes"
              placeholder="Search VS Code themes on Open VSX"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              className="py-1"
            />
          </div>
          <Button
            type="submit"
            variant="secondary"
            className="px-3 py-1 text-xs"
            loading={searching}
          >
            Search
          </Button>
        </form>
        <Button
          variant="ghost"
          className="px-3 py-1 text-xs"
          onClick={() => fileInput.current?.click()}
        >
          Import file…
        </Button>
        <input
          ref={fileInput}
          type="file"
          accept=".json,.vsix"
          aria-label="VS Code theme file"
          onChange={importFile}
          className="hidden"
        />
      </div>

      {error && (
        <p role="alert" className="text-xs text-danger select-text">
          {error}
        </p>
      )}

      {choice && (
        <div>
          <p className="text-xs text-text-muted mb-1.5">Themes in {choice.source}</p>
          <div className="flex flex-wrap gap-1.5">
            {choice.themes.map((theme, index) => (
              <button
                key={`${index}-${theme.name}`}
                onClick={() => onImport(theme.colors)}
                className="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs text-text-primary hover:bg-bg-hover cursor-pointer"
              >
                <span
                  className="w-3 h-3 rounded-full border border-border"
                  style={{ backgroundColor: theme.colors["bg-primary"] }}
                />
                {theme.name}
              </button>
            ))}
          </div>
        </div>
      )}

      {results && results.length === 0 && <EmptyState title="No themes found on Open VSX" />}
      {results && results.length > 0 && (
        <ul className="max-h-56 overflow-y-auto rounded-lg border border-border divide-y divide-border">
          {results.map((extension) => (
            <li key={extension.id}>
              <button
                onClick={() => loadExtension(extension)}
                disabled={loadingId !== null}
                className="w-full px-3 py-2 text-left hover:bg-bg-hover cursor-pointer disabled:cursor-default disabled:opacity-60"
              >
                <p className="text-sm text-text-primary truncate">
                  {extension.name}
                  <span className="ml-2 text-xs text-text-muted">{extension.publisher}</span>
                  {loadingId === extension.id && (
                    <span className="ml-2 text-xs text-text-muted">Loading…</span>
                  )}
                </p>
                {extension.description && (
                  <p className="text-xs text-text-muted truncate">{extension.description}</p>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
