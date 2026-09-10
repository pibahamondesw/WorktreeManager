import { useLayoutEffect } from "react";
import { editorPresentation } from "../services/codeEditor";

export function useEditorOcclusion(open: boolean) {
  useLayoutEffect(() => {
    if (open) return editorPresentation.suppress();
  }, [open]);
}
