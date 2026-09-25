import { useCallback, useState } from "react";

interface RevealRing {
  taskId: string;
  nonce: number;
}

export function useRevealRing() {
  const [ring, setRing] = useState<RevealRing | null>(null);

  const ringTask = useCallback(
    (taskId: string) => setRing((current) => ({ taskId, nonce: (current?.nonce ?? 0) + 1 })),
    []
  );

  const clearRing = useCallback(() => setRing(null), []);

  return { ring, ringTask, clearRing };
}
