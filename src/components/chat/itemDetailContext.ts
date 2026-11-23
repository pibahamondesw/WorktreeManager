import { createContext, useContext, useEffect } from "react";
import { DetailView } from "../../services/chat";

type ExpandDetail = (itemId: string, view: DetailView | null) => void;

export const ItemDetailContext = createContext<ExpandDetail>(() => undefined);

/** Receives a row's output from the session while `view` is set, the row being expanded. */
export function useExpandedDetail(itemId: string, view: DetailView | null) {
  const expand = useContext(ItemDetailContext);
  const expanded = view !== null;
  useEffect(() => {
    if (view) expand(itemId, view);
  }, [expand, itemId, view]);
  useEffect(() => {
    if (!expanded) return;
    return () => expand(itemId, null);
  }, [expand, itemId, expanded]);
}
