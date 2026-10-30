import { setLanguage } from "./i18n";
import { useCallback, useEffect, useRef, useState } from "react";
import { execute, snapshot } from "./api";
import type { Params, Snapshot } from "./types";
// useSession 用串行轮询同步多窗口快照，清理时停止所有定时器。
export function useSession() {
  const [state, setState] = useState<Snapshot | null>(null);
  const [error, setError] = useState("");
  const revision = useRef(-1);
  const active = useRef(true);
  const refresh = useCallback(async () => {
    const next = await snapshot();
    setLanguage(next.prefs.language);
    if (active.current && revision.current < next.revision) {
      revision.current = next.revision;
      setState(next);
    }
    return next;
  }, []);
  useEffect(() => {
    active.current = true;
    let timer: ReturnType<typeof setTimeout>;
    let cancelled = false;
    const poll = async () => {
      try {
        await refresh();
      } catch (e) {
        if (active.current) setError(String(e));
      }
      if (!cancelled) timer = setTimeout(poll, 500);
    };
    void poll();
    return () => {
      active.current = false;
      cancelled = true;
      clearTimeout(timer);
    };
  }, [refresh]);
  const run = useCallback(
    async <T = unknown>(
      id: string,
      params: Params = {},
    ): Promise<T | undefined> => {
      setError("");
      try {
        const result = await execute<T>(id, params);
        await refresh();
        return result;
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        return undefined;
      }
    },
    [refresh],
  );
  return { state, error, setError, run, refresh };
}
export type SessionUI = ReturnType<typeof useSession>;
