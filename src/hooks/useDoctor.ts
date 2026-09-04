import { useCallback, useEffect, useRef, useState } from "react";
import { DoctorConfig, DoctorReport, runDoctor } from "../services/doctor";

/**
 * Runs the dependency check once the store has loaded, then only on request. Config changes
 * (adding a workspace, switching editor) don't retrigger it: the check shells out and hits the
 * network, and it is advisory, so re-running belongs to the user's "Re-check".
 *
 * Pass `null` while the app isn't ready to be checked. A check that throws is logged and
 * dropped — the doctor must never be louder than the problems it looks for.
 */
export function useDoctor(config: DoctorConfig | null) {
  const [report, setReport] = useState<DoctorReport | null>(null);
  const [running, setRunning] = useState(false);
  const configRef = useRef(config);
  const startedRef = useRef(false);

  useEffect(() => {
    configRef.current = config;
  }, [config]);

  const recheck = useCallback(async () => {
    const current = configRef.current;
    if (!current) return;
    setRunning(true);
    try {
      setReport(await runDoctor(current));
    } catch (e) {
      console.error("WorktreeManager: dependency check failed", e);
    } finally {
      setRunning(false);
    }
  }, []);

  useEffect(() => {
    if (startedRef.current || !config) return;
    startedRef.current = true;
    void recheck();
  }, [config, recheck]);

  return { report, running, recheck };
}
