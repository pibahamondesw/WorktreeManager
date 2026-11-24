import {
  ArrowUpRight,
  Bell,
  Bot,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  CircleCheck,
  CircleExclamationPoint,
  Copy,
  Download,
  EllipsisVertical,
  File,
  FileArchive,
  FileBraces,
  FileCode,
  FileCog,
  FileDatabase,
  FileImage,
  FileKey,
  FileLock,
  FileMusic,
  FileSpreadsheet,
  FileTerminal,
  FileText,
  FileVideoCamera,
  Folder,
  GitBranch,
  GitMerge,
  GitPullRequest,
  GitPullRequestClosed,
  GitPullRequestDraft,
  GripVertical,
  Maximize2,
  Notebook,
  PanelLeft,
  Pencil,
  Pin,
  Plus,
  Redo2,
  RefreshCw,
  Scan,
  Search,
  Settings,
  SquareCode,
  Sun,
  Trash,
  TriangleExclamationPoint,
  Undo2,
  Wrench,
  X,
  Zap,
  type LucideIcon,
} from "lucide-react";
import { createElement } from "react";
import { agentPresentation } from "../../services/agents";
import { AgentId } from "../../types";

interface IconProps {
  size?: number;
  className?: string;
}

const remSize = (size: number) => `${size / 16}rem`;

function libraryIcon(Glyph: LucideIcon, defaultSize: number) {
  return function LibraryIcon({ size = defaultSize, className = "" }: IconProps) {
    return <Glyph size={remSize(size)} className={className} aria-hidden="true" />;
  };
}

export const RewindIcon = libraryIcon(Undo2, 16);
/** A curved arrow turning forward: redirect what is running. */
export const SteerIcon = libraryIcon(Redo2, 12);
export const CheckIcon = libraryIcon(Check, 12);
export const PlusIcon = libraryIcon(Plus, 12);
export const CloseIcon = libraryIcon(X, 16);
export const SearchIcon = libraryIcon(Search, 16);
export const ChevronDownIcon = libraryIcon(ChevronDown, 10);
export const ChevronLeftIcon = libraryIcon(ChevronLeft, 14);
export const ChevronRightIcon = libraryIcon(ChevronRight, 14);
export const ExternalLinkIcon = libraryIcon(ArrowUpRight, 10);
export const CopyIcon = libraryIcon(Copy, 12);
export const PencilIcon = libraryIcon(Pencil, 12);
export const TrashIcon = libraryIcon(Trash, 16);
export const RefreshIcon = libraryIcon(RefreshCw, 14);
export const MoreVerticalIcon = libraryIcon(EllipsisVertical, 14);
export const GripIcon = libraryIcon(GripVertical, 12);
export const BranchIcon = libraryIcon(GitBranch, 12);
export const PullRequestIcon = libraryIcon(GitPullRequest, 12);
export const FolderIcon = libraryIcon(Folder, 24);
export const WarningCircleIcon = libraryIcon(CircleExclamationPoint, 10);
export const SuccessCircleIcon = libraryIcon(CircleCheck, 16);
export const SunIcon = libraryIcon(Sun, 14);
export const NotebookIcon = libraryIcon(Notebook, 14);
export const CodeBranchIcon = libraryIcon(SquareCode, 24);
export const GearIcon = libraryIcon(Settings, 14);
export const AutomationIcon = libraryIcon(Zap, 14);
export const WrenchIcon = libraryIcon(Wrench, 32);
export const SidebarIcon = libraryIcon(PanelLeft, 14);
export const BellIcon = libraryIcon(Bell, 14);
export const PinIcon = libraryIcon(Pin, 16);
export const ExpandIcon = libraryIcon(Maximize2, 14);
export const FitIcon = libraryIcon(Scan, 14);
export const DownloadIcon = libraryIcon(Download, 14);
export const DocumentIcon = libraryIcon(FileText, 16);
export const BotIcon = libraryIcon(Bot, 14);

export function SpinnerIcon({ size = 16, className = "" }: IconProps) {
  return (
    <svg
      className={`motion-spin-steps ${className}`}
      width={remSize(size)}
      height={remSize(size)}
      viewBox="0 0 24 24"
      fill="none"
    >
      <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
      <path
        className="opacity-75"
        fill="currentColor"
        d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
      />
    </svg>
  );
}

export function FitWidthIcon({ size = 14, className = "" }: IconProps) {
  return (
    <svg
      width={remSize(size)}
      height={remSize(size)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      <path d="M3 4v16M21 4v16M7 12h10M10 9l-3 3 3 3M14 9l3 3-3 3" />
    </svg>
  );
}

export type PullRequestDisplayState = "open" | "draft" | "merged" | "closed" | "conflicts";

const PULL_REQUEST_STATE_GLYPHS: Record<
  PullRequestDisplayState,
  { Glyph: LucideIcon; className: string }
> = {
  open: { Glyph: GitPullRequest, className: "text-success" },
  draft: { Glyph: GitPullRequestDraft, className: "text-text-muted" },
  merged: { Glyph: GitMerge, className: "text-accent" },
  closed: { Glyph: GitPullRequestClosed, className: "text-danger" },
  conflicts: { Glyph: TriangleExclamationPoint, className: "text-warning" },
};

export function PullRequestStateIcon({
  state,
  size = 12,
  className = "",
}: IconProps & { state: PullRequestDisplayState }) {
  const { Glyph, className: stateClassName } = PULL_REQUEST_STATE_GLYPHS[state];
  return (
    <Glyph
      size={remSize(size)}
      className={`${stateClassName} ${className}`}
      aria-hidden="true"
      data-state={state}
    />
  );
}

export function AgentIcon({ agent, size = 12, className = "" }: IconProps & { agent: AgentId }) {
  const Glyph = agentPresentation(agent).icon ?? Bot;
  return <Glyph size={remSize(size)} className={className} aria-hidden="true" />;
}

const GLYPHS_BY_FILE_NAME: Record<string, LucideIcon> = {
  "cargo.lock": FileLock,
  "package-lock.json": FileLock,
  "pnpm-lock.yaml": FileLock,
  "yarn.lock": FileLock,
  dockerfile: FileCog,
  makefile: FileCog,
};

const EXTENSIONS_BY_GLYPH: [LucideIcon, string[]][] = [
  [FileLock, ["lock"]],
  [
    FileCode,
    [
      ...["c", "cc", "cjs", "cpp", "cs", "css", "go", "h", "hpp", "html", "java", "js", "jsx"],
      ...["kt", "mjs", "php", "py", "rb", "rs", "scss", "svelte", "swift", "ts", "tsx", "vue"],
    ],
  ],
  [FileBraces, ["json", "jsonc", "toml", "xml", "yaml", "yml"]],
  [FileText, ["md", "mdx", "rst", "txt"]],
  [FileTerminal, ["bash", "fish", "sh", "zsh"]],
  [FileImage, ["bmp", "gif", "ico", "jpeg", "jpg", "png", "svg", "webp"]],
  [FileSpreadsheet, ["csv", "tsv", "xlsx"]],
  [FileArchive, ["gz", "tar", "tgz", "zip"]],
  [FileDatabase, ["db", "sql", "sqlite"]],
  [FileVideoCamera, ["mov", "mp4", "webm"]],
  [FileMusic, ["mp3", "wav"]],
];

const GLYPH_BY_EXTENSION = new Map(
  EXTENSIONS_BY_GLYPH.flatMap(([glyph, extensions]) =>
    extensions.map((extension) => [extension, glyph] as const)
  )
);

function fileTypeGlyph(path: string): LucideIcon {
  const name = path.slice(path.lastIndexOf("/") + 1).toLowerCase();
  if (Object.hasOwn(GLYPHS_BY_FILE_NAME, name)) return GLYPHS_BY_FILE_NAME[name];
  if (name === ".env" || name.startsWith(".env.")) return FileKey;
  const dot = name.lastIndexOf(".");
  if (dot === 0) return FileCog;
  return (dot > 0 && GLYPH_BY_EXTENSION.get(name.slice(dot + 1))) || File;
}

export function FileTypeIcon({ path, size = 12, className = "" }: IconProps & { path: string }) {
  return createElement(fileTypeGlyph(path), {
    size: remSize(size),
    className,
    "aria-hidden": "true",
  });
}
