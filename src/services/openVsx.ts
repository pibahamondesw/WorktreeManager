import { ImportedTheme } from "../vscodeTheme";
import { themesFromVsix } from "../vsixThemes";

export interface OpenVsxExtension {
  id: string;
  name: string;
  publisher: string;
  description: string;
  downloadUrl: string;
}

const OPEN_VSX_ORIGIN = "https://open-vsx.org";
const REQUEST_TIMEOUT_MS = 20_000;
const MAX_PACKAGE_BYTES = 25 * 1024 * 1024;
const UNREACHABLE = "Could not reach Open VSX. Check your connection and try again.";

async function request(url: string): Promise<Response> {
  let response: Response;
  try {
    response = await fetch(url, { signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS) });
  } catch {
    throw new Error(UNREACHABLE);
  }
  if (!response.ok) throw new Error(`Open VSX responded with ${response.status} for ${url}`);
  return response;
}

function text(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function extensionFromResult(value: unknown): OpenVsxExtension | null {
  if (value === null || typeof value !== "object") return null;
  const result = value as Record<string, unknown>;
  const files = (result.files ?? {}) as Record<string, unknown>;
  const downloadUrl = text(files.download);
  const namespace = text(result.namespace);
  const name = text(result.name);
  if (!namespace || !name || !downloadUrl.startsWith(`${OPEN_VSX_ORIGIN}/`)) return null;
  return {
    id: `${namespace}.${name}`,
    name: text(result.displayName) || name,
    publisher: namespace,
    description: text(result.description),
    downloadUrl,
  };
}

/** Theme extensions on Open VSX matching `query`, most downloaded first. */
export async function searchOpenVsxThemes(query: string): Promise<OpenVsxExtension[]> {
  const url = new URL("/api/-/search", OPEN_VSX_ORIGIN);
  url.search = new URLSearchParams({
    query: query.trim(),
    category: "Themes",
    sortBy: "downloadCount",
    sortOrder: "desc",
    size: "20",
  }).toString();
  const response = await request(url.toString());
  const body = (await response.json().catch(() => null)) as { extensions?: unknown } | null;
  if (!Array.isArray(body?.extensions))
    throw new Error("Open VSX returned an unreadable response.");
  return body.extensions.map(extensionFromResult).filter((e): e is OpenVsxExtension => e !== null);
}

/** Downloads an extension package and reads the color themes it contributes. */
export async function loadOpenVsxThemes(extension: OpenVsxExtension): Promise<ImportedTheme[]> {
  const tooLarge = new Error(`${extension.name} is too large to import (over 25 MB).`);
  const response = await request(extension.downloadUrl);
  if (Number(response.headers.get("content-length")) > MAX_PACKAGE_BYTES) throw tooLarge;
  const buffer = await response.arrayBuffer().catch(() => {
    throw new Error(UNREACHABLE);
  });
  if (buffer.byteLength > MAX_PACKAGE_BYTES) throw tooLarge;
  return themesFromVsix(new Uint8Array(buffer));
}
