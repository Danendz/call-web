import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { ROOM_ID_RE } from "../../shared/protocol";
import { CallController, type CallState } from "./call/controller";
import type { CallStats } from "./call/stats";
import { ChatPanel } from "./chat/ChatPanel";

const ROOM_ALPHABET = "abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";

function resolveRoomId(): string {
  const match = location.pathname.match(/^\/r\/([^/]+)\/?$/);
  if (match && ROOM_ID_RE.test(match[1])) return match[1];
  const bytes = crypto.getRandomValues(new Uint8Array(10));
  const id = Array.from(bytes, (b) => ROOM_ALPHABET[b % ROOM_ALPHABET.length]).join("");
  history.replaceState(null, "", `/r/${id}${location.search}`);
  return id;
}

export function App() {
  const roomId = useMemo(resolveRoomId, []);
  const controller = useMemo(() => new CallController(roomId), [roomId]);
  const state = useSyncExternalStore(controller.subscribe, controller.getState);
  const roomUrl = `${location.origin}/r/${roomId}`;

  switch (state.phase) {
    case "idle":
    case "error":
      return (
        <Centered>
          <h1>Video call</h1>
          <p className="muted">Share this link with the person you want to call:</p>
          <CopyLink url={roomUrl} />
          {state.error && <p className="error">{state.error}</p>}
          <button className="primary" onClick={() => controller.start()}>
            Join call
          </button>
        </Centered>
      );
    case "full":
      return (
        <Centered>
          <h1>Room is full</h1>
          <p className="muted">This room already has two people in it.</p>
          <a className="button primary" href="/">
            Start a new room
          </a>
        </Centered>
      );
    case "ended":
      return (
        <Centered>
          <h1>Call ended</h1>
          <button className="primary" onClick={() => controller.start()}>
            Rejoin
          </button>
          <a className="button" href="/">
            New room
          </a>
        </Centered>
      );
    default:
      return <CallView controller={controller} state={state} roomUrl={roomUrl} />;
  }
}

function CallView({ controller, state, roomUrl }: { controller: CallController; state: CallState; roomUrl: string }) {
  const [chatOpen, setChatOpen] = useState(() => window.matchMedia("(min-width: 900px)").matches);
  const [showStats, setShowStats] = useState(state.forceRelay);
  const [seenCount, setSeenCount] = useState(0);
  const canShare = typeof navigator.mediaDevices?.getDisplayMedia === "function";

  const othersCount = state.chat.filter((m) => m.from !== state.myId).length;
  useEffect(() => {
    if (chatOpen) setSeenCount(othersCount);
  }, [chatOpen, othersCount]);
  const unread = othersCount - seenCount;

  const banner = statusBanner(state);

  return (
    <div className={`call ${chatOpen ? "chat-open" : ""}`}>
      <div className="stage">
        {state.phase === "in-call" && state.remoteStream ? (
          <Video stream={state.remoteStream} className="remote" />
        ) : (
          <div className="waiting">
            <p>{state.phase === "starting" ? "Starting…" : "Waiting for the other person to join…"}</p>
            <CopyLink url={roomUrl} />
          </div>
        )}
        {state.localStream && (
          <Video stream={state.localStream} className={`local ${state.sharing ? "screen" : ""}`} muted mirror={!state.sharing} />
        )}
        {banner && <div className="banner">{banner}</div>}
        {showStats && (
          <StatsOverlay stats={state.stats} forceRelay={state.forceRelay} onForceRelay={(v) => controller.setForceRelay(v)} />
        )}
      </div>

      {chatOpen && (
        <ChatPanel messages={state.chat} myId={state.myId} onSend={(t) => controller.sendChat(t)} onClose={() => setChatOpen(false)} />
      )}

      <div className="controls">
        <button className={state.micOn ? "" : "off"} onClick={() => controller.toggleMic()}>
          {state.micOn ? "Mute" : "Unmute"}
        </button>
        <button className={state.camOn ? "" : "off"} disabled={!state.hasCamera} onClick={() => controller.toggleCam()}>
          {state.camOn ? "Camera off" : "Camera on"}
        </button>
        {canShare && (
          <button className={state.sharing ? "active" : ""} disabled={state.phase !== "in-call"} onClick={() => controller.toggleScreenShare()}>
            {state.sharing ? "Stop sharing" : "Share screen"}
          </button>
        )}
        <button className={showStats ? "active" : ""} onClick={() => setShowStats((v) => !v)}>
          Stats
        </button>
        <button className={chatOpen ? "active" : ""} onClick={() => setChatOpen((v) => !v)}>
          Chat{unread > 0 && !chatOpen ? <span className="badge">{unread}</span> : null}
        </button>
        <button className="danger" onClick={() => controller.hangUp()}>
          Hang up
        </button>
      </div>
    </div>
  );
}

function statusBanner(state: CallState): string | null {
  if (state.socket === "reconnecting") return "Lost connection to server, reconnecting…";
  if (state.phase !== "in-call") return null;
  switch (state.iceState) {
    case "new":
    case "checking":
      return "Connecting…";
    case "disconnected":
      return "Connection unstable, reconnecting…";
    case "failed":
      return "Connection failed, retrying…";
    default:
      return null;
  }
}

function Video({ stream, className, muted, mirror }: { stream: MediaStream; className: string; muted?: boolean; mirror?: boolean }) {
  const ref = useRef<HTMLVideoElement>(null);
  useEffect(() => {
    if (ref.current && ref.current.srcObject !== stream) ref.current.srcObject = stream;
  }, [stream]);
  return <video ref={ref} className={`${className} ${mirror ? "mirror" : ""}`} autoPlay playsInline muted={muted} />;
}

function StatsOverlay({ stats, forceRelay, onForceRelay }: { stats: CallStats | null; forceRelay: boolean; onForceRelay: (v: boolean) => void }) {
  const rttClass = stats?.rttMs === undefined ? "" : stats.rttMs < 150 ? "good" : stats.rttMs < 300 ? "ok" : "bad";
  const path =
    !stats || stats.path === "unknown"
      ? "—"
      : stats.path === "relay"
        ? `TURN relay (${stats.relayProtocol ?? stats.protocol ?? "?"})`
        : `Direct P2P (${stats.localType} ↔ ${stats.remoteType}, ${stats.protocol})`;
  return (
    <div className="stats">
      <dl>
        <dt>Path</dt>
        <dd>{path}</dd>
        <dt>RTT</dt>
        <dd className={rttClass}>{fmt(stats?.rttMs, "ms")}</dd>
        <dt>Loss</dt>
        <dd>{fmt(stats?.lossPct, "%", 1)}</dd>
        <dt>Jitter</dt>
        <dd>{fmt(stats?.jitterMs, "ms")}</dd>
        <dt>Down / Up</dt>
        <dd>
          {fmt(stats?.inKbps, "")} / {fmt(stats?.outKbps, "kbps")}
        </dd>
        <dt>Video in</dt>
        <dd>{stats?.video?.width ? `${stats.video.width}×${stats.video.height} @ ${fmt(stats.video.fps, "fps")}` : "—"}</dd>
      </dl>
      <label>
        <input type="checkbox" checked={forceRelay} onChange={(e) => onForceRelay(e.target.checked)} /> Force TURN relay
      </label>
    </div>
  );
}

function fmt(value: number | undefined, unit: string, digits = 0): string {
  return value === undefined ? "—" : `${value.toFixed(digits)}${unit ? ` ${unit}` : ""}`;
}

function CopyLink({ url }: { url: string }) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // clipboard blocked (http / old browser): the link is still selectable
    }
  };
  return (
    <div className="copy-link">
      <input readOnly value={url} onFocus={(e) => e.target.select()} />
      <button onClick={copy}>{copied ? "Copied" : "Copy"}</button>
    </div>
  );
}

function Centered({ children }: { children: React.ReactNode }) {
  return <div className="centered">{children}</div>;
}
