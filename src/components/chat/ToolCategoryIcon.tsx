import {
  FileEditIcon,
  GlobeIcon,
  ReadIcon,
  SearchIcon,
  TerminalIcon,
  WrenchIcon,
} from "../ui/Icons";
import { ToolCategory } from "./toolCategory";

const ICON_SIZE = 12;

const CATEGORY_ICON: Record<ToolCategory, typeof WrenchIcon> = {
  edit: FileEditIcon,
  command: TerminalIcon,
  read: ReadIcon,
  search: SearchIcon,
  web: GlobeIcon,
  other: WrenchIcon,
};

export function ToolCategoryIcon({ category }: { category: ToolCategory }) {
  const Icon = CATEGORY_ICON[category];
  return <Icon size={ICON_SIZE} className="shrink-0 text-text-muted" />;
}
