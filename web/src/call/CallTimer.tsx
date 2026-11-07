import { useEffect, useState } from "react";

/** 4:05, or 1:04:05 past an hour. */
export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

/** How long the call has run, at the top of the stage. startedAt is a performance.now() time. */
export function CallTimer({ startedAt }: { startedAt: number }) {
  const [now, setNow] = useState(() => performance.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(performance.now()), 1000);
    return () => clearInterval(id);
  }, []);
  return <div className="call-timer">{formatDuration(now - startedAt)}</div>;
}
