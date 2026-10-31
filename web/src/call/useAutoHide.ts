import { useCallback, useEffect, useRef, useState } from "react";

const IDLE_MS = 3_000;

/** Idle after a few seconds without input, so the controls can fade off the video. Never idle while disabled. */
export function useAutoHide(enabled: boolean): { idle: boolean; wake: () => void; sleep: () => void } {
  const [idle, setIdle] = useState(false);
  const timer = useRef<number>();

  const wake = useCallback(() => {
    setIdle(false);
    clearTimeout(timer.current);
    if (enabled) timer.current = window.setTimeout(() => setIdle(true), IDLE_MS);
  }, [enabled]);

  const sleep = useCallback(() => {
    clearTimeout(timer.current);
    if (enabled) setIdle(true);
  }, [enabled]);

  useEffect(() => {
    wake();
    return () => clearTimeout(timer.current);
  }, [wake]);

  return { idle: enabled && idle, wake, sleep };
}
