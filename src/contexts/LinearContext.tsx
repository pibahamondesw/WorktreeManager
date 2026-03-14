import { createContext, useContext, useMemo, type ReactNode } from "react";
import { LinearService } from "../services/linear";

const LinearContext = createContext<LinearService | null>(null);

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

export function useLinear(): LinearService | null {
  return useContext(LinearContext);
}
