import {
  isValidElement,
  KeyboardEvent,
  MouseEvent,
  ReactNode,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { FileTypeIcon } from "../ui/Icons";
import { FileReference, fileLinkCopies, fileLinkLabel, PathCopy } from "./markdownDocument";

const VIEWPORT_MARGIN = 8;

interface MenuPosition {
  top: number;
  left: number;
}

function plainText(node: ReactNode): string {
  if (typeof node === "string") return node;
  if (Array.isArray(node)) return node.map(plainText).join("");
  if (isValidElement<{ children?: ReactNode }>(node)) return plainText(node.props.children);
  return "";
}

const withinViewport = (start: number, size: number, viewport: number) =>
  Math.max(VIEWPORT_MARGIN, Math.min(start, viewport - size - VIEWPORT_MARGIN));

function PathCopyMenu({
  id,
  position,
  copies,
  onPick,
  onDismiss,
  onClose,
}: {
  id: string;
  position: MenuPosition;
  copies: PathCopy[];
  onPick: (value: string) => void;
  onDismiss: () => void;
  onClose: () => void;
}) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [placed, setPlaced] = useState(position);

  useLayoutEffect(() => {
    const rect = menuRef.current!.getBoundingClientRect();
    setPlaced({
      left: withinViewport(position.left, rect.width, window.innerWidth),
      top: withinViewport(position.top, rect.height, window.innerHeight),
    });
  }, [position]);

  useEffect(() => {
    menuRef.current?.querySelector("button")?.focus();
    const dismissOutside = (event: PointerEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) onDismiss();
    };
    document.addEventListener("pointerdown", dismissOutside);
    window.addEventListener("resize", onDismiss);
    window.addEventListener("scroll", onDismiss, true);
    return () => {
      document.removeEventListener("pointerdown", dismissOutside);
      window.removeEventListener("resize", onDismiss);
      window.removeEventListener("scroll", onDismiss, true);
    };
  }, [onDismiss]);

  const navigate = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    if (event.key === "Tab") {
      onClose();
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    const buttons = [...menuRef.current!.querySelectorAll("button")];
    const active = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const step = event.key === "ArrowDown" ? 1 : -1;
    buttons[(active + step + buttons.length) % buttons.length].focus();
  };

  return createPortal(
    <div
      ref={menuRef}
      id={id}
      role="menu"
      tabIndex={-1}
      aria-label="File path"
      onKeyDown={navigate}
      style={placed}
      className="fixed z-50 rounded-lg border border-border bg-bg-secondary p-1 shadow-xl"
    >
      {copies.map((copy) => (
        <button
          key={copy.label}
          role="menuitem"
          type="button"
          onClick={() => onPick(copy.value)}
          className="block w-full rounded-md px-3 py-1.5 text-left text-xs text-text-primary hover:bg-bg-hover focus:bg-bg-hover focus:outline-none cursor-pointer"
        >
          {copy.label}
        </button>
      ))}
    </div>,
    document.body
  );
}

export function MarkdownFileLink({
  href,
  reference,
  folders,
  children,
  onOpen,
}: {
  href?: string;
  children?: ReactNode;
  reference: FileReference;
  folders: string[];
  onOpen: () => void;
}) {
  const [menuAt, setMenuAt] = useState<MenuPosition | null>(null);
  const [copyError, setCopyError] = useState<string | null>(null);
  const linkRef = useRef<HTMLAnchorElement>(null);
  const menuId = useId();
  const dismissMenu = useCallback(() => setMenuAt(null), []);
  const closeMenu = () => {
    setMenuAt(null);
    linkRef.current?.focus();
  };
  const open = (event: MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault();
    onOpen();
  };
  const showMenuBelowLink = () => {
    const rect = linkRef.current!.getBoundingClientRect();
    setMenuAt({ left: rect.left, top: rect.bottom + 4 });
  };
  const showMenuAtPointer = (event: MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault();
    setMenuAt({ left: event.clientX, top: event.clientY });
  };
  const showMenuFromKeyboard = (event: KeyboardEvent<HTMLAnchorElement>) => {
    if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
    event.preventDefault();
    showMenuBelowLink();
  };
  const copy = (value: string) => {
    closeMenu();
    setCopyError(null);
    navigator.clipboard.writeText(value).catch(() => setCopyError("Could not copy the path"));
  };

  return (
    <>
      <a
        ref={linkRef}
        href={href}
        title={reference.path}
        aria-haspopup="menu"
        aria-expanded={menuAt !== null}
        aria-controls={menuAt ? menuId : undefined}
        onClick={open}
        onAuxClick={(event) => {
          if (event.button === 1) open(event);
        }}
        onContextMenu={showMenuAtPointer}
        onKeyDown={showMenuFromKeyboard}
        className="chat-file-link"
      >
        <FileTypeIcon path={reference.path} className="shrink-0 text-text-muted" />
        <span className="truncate">{fileLinkLabel(reference, folders, plainText(children))}</span>
      </a>
      {menuAt && (
        <PathCopyMenu
          id={menuId}
          position={menuAt}
          copies={fileLinkCopies(reference, folders)}
          onPick={copy}
          onDismiss={dismissMenu}
          onClose={closeMenu}
        />
      )}
      {copyError && (
        <span role="alert" className="text-danger">
          {" "}
          {copyError}
        </span>
      )}
    </>
  );
}
