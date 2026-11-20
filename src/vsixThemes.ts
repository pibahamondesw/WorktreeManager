import {
  colorsFromVscodeTheme,
  ImportedTheme,
  parseThemeDocument,
  themeName,
  VscodeThemeDocument,
} from "./vscodeTheme";

interface ZipEntry {
  method: number;
  compressedSize: number;
  localHeaderOffset: number;
}

const END_OF_CENTRAL_DIRECTORY = 0x06054b50;
const CENTRAL_DIRECTORY_ENTRY = 0x02014b50;
const LOCAL_FILE_HEADER = 0x04034b50;
const STORED = 0;
const DEFLATED = 8;
const MAX_INCLUDE_DEPTH = 5;

const UNREADABLE_PACKAGE = "This extension package is not a readable VSIX file.";

function zipEntries(bytes: Uint8Array): Map<string, ZipEntry> {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let end = bytes.length - 22;
  const earliest = Math.max(0, bytes.length - 22 - 0xffff);
  while (end >= earliest && view.getUint32(end, true) !== END_OF_CENTRAL_DIRECTORY) end -= 1;
  if (end < earliest) throw new Error(UNREADABLE_PACKAGE);
  const count = view.getUint16(end + 10, true);
  let offset = view.getUint32(end + 16, true);
  const decoder = new TextDecoder();
  const entries = new Map<string, ZipEntry>();
  for (let i = 0; i < count; i += 1) {
    if (offset + 46 > bytes.length || view.getUint32(offset, true) !== CENTRAL_DIRECTORY_ENTRY) {
      throw new Error(UNREADABLE_PACKAGE);
    }
    const nameLength = view.getUint16(offset + 28, true);
    const name = decoder.decode(bytes.subarray(offset + 46, offset + 46 + nameLength));
    entries.set(name, {
      method: view.getUint16(offset + 10, true),
      compressedSize: view.getUint32(offset + 20, true),
      localHeaderOffset: view.getUint32(offset + 42, true),
    });
    offset +=
      46 + nameLength + view.getUint16(offset + 30, true) + view.getUint16(offset + 32, true);
  }
  return entries;
}

async function entryText(bytes: Uint8Array, entry: ZipEntry): Promise<string> {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const header = entry.localHeaderOffset;
  if (header + 30 > bytes.length || view.getUint32(header, true) !== LOCAL_FILE_HEADER) {
    throw new Error(UNREADABLE_PACKAGE);
  }
  const start = header + 30 + view.getUint16(header + 26, true) + view.getUint16(header + 28, true);
  const data = bytes.slice(start, start + entry.compressedSize);
  if (entry.method === STORED) return new TextDecoder().decode(data);
  if (entry.method !== DEFLATED) throw new Error(UNREADABLE_PACKAGE);
  const inflated = new ReadableStream<BufferSource>({
    start(controller) {
      controller.enqueue(data);
      controller.close();
    },
  }).pipeThrough(new DecompressionStream("deflate-raw"));
  return new Response(inflated).text().catch(() => {
    throw new Error(UNREADABLE_PACKAGE);
  });
}

function resolvePath(base: string, relative: string): string {
  const parts = base.split("/").slice(0, -1);
  for (const part of relative.split("/")) {
    if (part === "..") parts.pop();
    else if (part && part !== ".") parts.push(part);
  }
  return parts.join("/");
}

interface ThemeContribution {
  label?: unknown;
  uiTheme?: unknown;
  path?: unknown;
}

/** Every color theme an extension package (`.vsix`) contributes. */
export async function themesFromVsix(bytes: Uint8Array): Promise<ImportedTheme[]> {
  const entries = zipEntries(bytes);
  const readJson = async (path: string): Promise<VscodeThemeDocument> => {
    const entry = entries.get(path);
    if (!entry) throw new Error(`The extension package is missing ${path}.`);
    return parseThemeDocument(await entryText(bytes, entry), path);
  };
  const readTheme = async (path: string, depth: number): Promise<VscodeThemeDocument> => {
    const theme = await readJson(path);
    if (typeof theme.include !== "string" || depth >= MAX_INCLUDE_DEPTH) return theme;
    const included = await readTheme(resolvePath(path, theme.include), depth + 1);
    const colors = (value: unknown) => (value !== null && typeof value === "object" ? value : {});
    return {
      ...included,
      ...theme,
      colors: { ...colors(included.colors), ...colors(theme.colors) },
    };
  };

  const manifest = (await readJson("extension/package.json")) as {
    contributes?: { themes?: unknown };
  };
  const contributions = Array.isArray(manifest.contributes?.themes)
    ? (manifest.contributes.themes as ThemeContribution[])
    : [];
  const imported: ImportedTheme[] = [];
  for (const contribution of contributions) {
    if (typeof contribution?.path !== "string" || !contribution.path.endsWith(".json")) continue;
    const theme = await readTheme(resolvePath("extension/package.json", contribution.path), 0);
    imported.push({
      name: typeof contribution.label === "string" ? contribution.label : themeName(theme, "Theme"),
      colors: colorsFromVscodeTheme({ type: contribution.uiTheme, ...theme }),
    });
  }
  if (imported.length === 0) throw new Error("This extension has no JSON color themes.");
  return imported;
}
