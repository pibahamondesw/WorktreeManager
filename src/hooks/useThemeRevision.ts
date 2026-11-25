import { useEffect, useState } from "react";
import { THEME_CHANGE_EVENT } from "../themes";

export function useThemeRevision() {
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    const rerender = () => setRevision((current) => current + 1);
    window.addEventListener(THEME_CHANGE_EVENT, rerender);
    return () => window.removeEventListener(THEME_CHANGE_EVENT, rerender);
  }, []);
  return revision;
}
