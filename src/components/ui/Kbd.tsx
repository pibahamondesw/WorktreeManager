import { ReactNode } from "react";

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className="px-1 py-0.5 bg-bg-tertiary rounded font-mono">{children}</kbd>;
}
