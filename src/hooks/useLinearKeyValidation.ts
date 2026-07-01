import { useCallback, useRef, useState } from "react";
import { validateLinearToken } from "../services/linear";

/** Shared Linear API-key entry + validation state for the workspace add/edit modals. */
export function useLinearKeyValidation() {
  const [linearKey, setLinearKeyState] = useState("");
  const [linearValid, setLinearValid] = useState(false);
  const [linearUser, setLinearUser] = useState<string | null>(null);
  const [linearValidating, setLinearValidating] = useState(false);
  const [linearError, setLinearError] = useState<string | null>(null);
  const validatingKeyRef = useRef("");

  const runValidation = useCallback(async (key: string) => {
    const trimmed = key.trim();
    if (!trimmed) return;
    validatingKeyRef.current = trimmed;
    setLinearValid(false);
    setLinearUser(null);
    setLinearError(null);
    setLinearValidating(true);
    try {
      const result = await validateLinearToken(trimmed);
      if (validatingKeyRef.current !== trimmed) return;
      setLinearValid(result.valid);
      setLinearUser(result.name ?? null);
      if (!result.valid) setLinearError(result.error ?? "Validation failed");
    } catch {
      if (validatingKeyRef.current !== trimmed) return;
      setLinearError("Validation failed");
    } finally {
      if (validatingKeyRef.current === trimmed) setLinearValidating(false);
    }
  }, []);

  const setLinearKey = useCallback((key: string) => {
    setLinearKeyState(key);
    setLinearValid(false);
    setLinearUser(null);
    setLinearError(null);
    validatingKeyRef.current = "";
  }, []);

  const reset = useCallback(() => {
    setLinearKeyState("");
    setLinearValid(false);
    setLinearUser(null);
    setLinearValidating(false);
    setLinearError(null);
    validatingKeyRef.current = "";
  }, []);

  return {
    linearKey,
    setLinearKey,
    linearValid,
    linearUser,
    linearValidating,
    linearError,
    runValidation,
    reset,
  };
}
