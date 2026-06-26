import { useMemo, type ReactNode } from "react";
import { LinearService } from "../services/linear";
import { LinearContext } from "./useLinear";

interface LinearProviderProps {
  apiKey: string | null;
  children: ReactNode;
}

export function LinearProvider({ apiKey, children }: LinearProviderProps) {
  const service = useMemo(
    () => (apiKey ? new LinearService(apiKey) : null),
    [apiKey]
  );

  return (
    <LinearContext.Provider value={service}>{children}</LinearContext.Provider>
  );
}
