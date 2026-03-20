import { useState, useEffect, useRef, useCallback } from "react";

export function useEphemeralToast() {
  const [toast, setToast] = useState<string | null>(null);
  const toastTimer = useRef<number>(undefined!);

  useEffect(() => {
    return () => window.clearTimeout(toastTimer.current);
  }, []);

  const showToast = useCallback((msg: string) => {
    window.clearTimeout(toastTimer.current);
    setToast(msg);
    toastTimer.current = window.setTimeout(
      () => setToast(null),
      msg.length > 40 ? 4000 : 1500
    );
  }, []);

  return { toast, showToast };
}
