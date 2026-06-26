import { createContext, useContext } from "react";
import { LinearService } from "../services/linear";

export const LinearContext = createContext<LinearService | null>(null);

export function useLinear(): LinearService | null {
  return useContext(LinearContext);
}
