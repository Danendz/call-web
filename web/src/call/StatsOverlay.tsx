import type { CallStats } from "./stats";

export function StatsOverlay({
  open,
  stats,
  tryingDirect,
  forceRelay,
  onForceRelay,
}: {
  open: boolean;
  stats: CallStats | null;
  tryingDirect: boolean;
  forceRelay: boolean;
  onForceRelay: (v: boolean) => void;
}) {
  const rttClass = stats?.rttMs === undefined ? "" : stats.rttMs < 150 ? "good" : stats.rttMs < 300 ? "ok" : "bad";
  const path =
    !stats || stats.path === "unknown"
      ? "—"
      : stats.path === "relay"
        ? `TURN relay (${stats.relayProtocol ?? stats.protocol ?? "?"})`
        : `Direct P2P (${stats.localType} ↔ ${stats.remoteType}, ${stats.protocol})`;
  const pathNote = tryingDirect ? " · trying direct…" : "";
  return (
    <div className={`stats ${open ? "open" : ""}`} aria-hidden={!open}>
      <dl>
        <dt>Path</dt>
        <dd>
          {path}
          {pathNote}
        </dd>
        <dt>RTT</dt>
        <dd className={rttClass}>{fmt(stats?.rttMs, "ms")}</dd>
        <dt>Loss in / out</dt>
        <dd>
          {fmt(stats?.lossPct, "", 1)} / {fmt(stats?.lossOutPct, "%", 1)}
        </dd>
        <dt>Jitter</dt>
        <dd>{fmt(stats?.jitterMs, "ms")}</dd>
        <dt>Down / Up</dt>
        <dd>
          {fmt(stats?.inKbps, "")} / {fmt(stats?.outKbps, "kbps")}
        </dd>
        <dt>Send budget</dt>
        <dd>{fmt(stats?.sendBudgetKbps, "kbps")}</dd>
        <dt>Video in</dt>
        <dd>{fmtVideo(stats?.video)}</dd>
        <dt>Video out</dt>
        <dd>
          {fmtVideo(stats?.videoOut)}
          {stats?.videoOut?.limitedBy && <span className="bad"> ({stats.videoOut.limitedBy})</span>}
        </dd>
      </dl>
      <label>
        <input type="checkbox" checked={forceRelay} onChange={(e) => onForceRelay(e.target.checked)} /> Force TURN relay
      </label>
    </div>
  );
}

function fmtVideo(v: { width?: number; height?: number; fps?: number } | undefined): string {
  return v?.width ? `${v.width}×${v.height} @ ${fmt(v.fps, "fps")}` : "—";
}

function fmt(value: number | undefined, unit: string, digits = 0): string {
  return value === undefined ? "—" : `${value.toFixed(digits)}${unit ? ` ${unit}` : ""}`;
}
