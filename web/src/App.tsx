import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { ROOM_ID_RE } from "../../shared/protocol";
import { CallController, type CallState, type DeviceOption, type MediaAccess, type MediaKind } from "./call/controller";
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
      return <Setup controller={controller} state={state} roomUrl={roomUrl} />;
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

/** Pre-join screen: camera preview, mic level, device pickers and permission help. */
function Setup({ controller, state, roomUrl }: { controller: CallController; state: CallState; roomUrl: string }) {
  useEffect(() => {
    void controller.preview();
  }, [controller]);

  const pending = state.micAccess === "pending" || state.camAccess === "pending";
  const blocked = state.micAccess === "denied" || state.camAccess === "denied";
  const showVideo = state.localStream && state.camAccess === "granted" && state.camOn;
  const joinLabel =
    pending ? "Waiting for permission…"
    : state.micAccess !== "granted" && state.camAccess !== "granted" ? "Join without camera & mic"
    : "Join call";

  return (
    <div className="setup">
      <div className="preview">
        {showVideo ? (
          <Video stream={state.localStream!} className="preview-video" muted mirror />
        ) : (
          <div className="preview-off">{state.camAccess === "granted" ? "Camera is off" : `Camera: ${ACCESS_TEXT[state.camAccess]}`}</div>
        )}
        {state.localStream && state.micAccess === "granted" && <MicLevel stream={state.localStream} />}
      </div>

      <div className="setup-panel">
        <h1>Ready to join?</h1>
        <p className="muted">Share this link with the person you want to call:</p>
        <CopyLink url={roomUrl} />

        <DeviceRow
          name="Microphone"
          kind="audio"
          access={state.micAccess}
          on={state.micOn}
          devices={state.mics}
          selected={state.micDeviceId}
          onToggle={() => controller.toggleMic()}
          onSelect={(kind, id) => controller.selectDevice(kind, id)}
        />
        <DeviceRow
          name="Camera"
          kind="video"
          access={state.camAccess}
          on={state.camOn}
          devices={state.cams}
          selected={state.camDeviceId}
          onToggle={() => controller.toggleCam()}
          onSelect={(kind, id) => controller.selectDevice(kind, id)}
        />

        {blocked && (
          <div className="permission-help">
            <p>
              The browser blocked access. Tap the camera or lock icon in the address bar, allow camera and microphone, then
              press Retry. On iPhone: Settings → Safari → Camera / Microphone → Allow.
            </p>
            <button onClick={() => controller.preview()}>Retry</button>
          </div>
        )}

        <button className="primary join" disabled={pending} onClick={() => controller.start()}>
          {joinLabel}
        </button>
      </div>
    </div>
  );
}

const ACCESS_TEXT: Record<MediaAccess, string> = {
  granted: "",
  pending: "Asking for permission…",
  denied: "Blocked by the browser",
  missing: "Not found",
  busy: "In use by another app",
};

function DeviceRow(props: {
  name: string;
  kind: MediaKind;
  access: MediaAccess;
  on: boolean;
  devices: DeviceOption[];
  selected: string | null;
  onToggle: () => void;
  onSelect: (kind: MediaKind, deviceId: string) => void;
}) {
  const granted = props.access === "granted";
  return (
    <div className="device-row">
      <button className={granted && !props.on ? "off" : ""} disabled={!granted} onClick={props.onToggle}>
        {props.name} {granted ? (props.on ? "on" : "off") : ""}
      </button>
      {granted ? (
        <select
          value={props.selected ?? ""}
          disabled={props.devices.length < 2}
          onChange={(e) => props.onSelect(props.kind, e.target.value)}
          aria-label={`${props.name} device`}
        >
          {props.devices.length === 0 && <option value="">Default</option>}
          {props.devices.map((d) => (
            <option key={d.id} value={d.id}>
              {d.label}
            </option>
          ))}
        </select>
      ) : (
        <span className={props.access === "pending" ? "muted" : "error"}>{ACCESS_TEXT[props.access]}</span>
      )}
    </div>
  );
}

/** Live input level so you can see the right mic is picked up before joining. */
function MicLevel({ stream }: { stream: MediaStream }) {
  const barRef = useRef<HTMLDivElement>(null);
  const track = stream.getAudioTracks()[0];
  useEffect(() => {
    if (!track) return;
    const ctx = new AudioContext();
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 512;
    ctx.createMediaStreamSource(new MediaStream([track])).connect(analyser);
    const samples = new Uint8Array(analyser.fftSize);
    let frame = 0;
    const tick = () => {
      analyser.getByteTimeDomainData(samples);
      let peak = 0;
      for (const v of samples) peak = Math.max(peak, Math.abs(v - 128));
      if (barRef.current) barRef.current.style.transform = `scaleX(${Math.min(1, peak / 64)})`;
      frame = requestAnimationFrame(tick);
    };
    tick();
    // Browsers may start the context suspended until the first tap.
    const resume = () => void ctx.resume();
    resume();
    window.addEventListener("pointerdown", resume);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("pointerdown", resume);
      void ctx.close();
    };
  }, [track]);
  return (
    <div className="mic-level" aria-hidden>
      <div ref={barRef} />
    </div>
  );
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
          <StatsOverlay
            stats={state.stats}
            tryingDirect={state.tryingDirect}
            forceRelay={state.forceRelay}
            onForceRelay={(v) => controller.setForceRelay(v)}
          />
        )}
      </div>

      {chatOpen && (
        <ChatPanel messages={state.chat} myId={state.myId} onSend={(t) => controller.sendChat(t)} onClose={() => setChatOpen(false)} />
      )}

      <div className="controls">
        <button className={state.micOn ? "" : "off"} disabled={state.micAccess !== "granted"} onClick={() => controller.toggleMic()}>
          {state.micOn ? "Mute" : "Unmute"}
        </button>
        <button className={state.camOn ? "" : "off"} disabled={state.camAccess !== "granted"} onClick={() => controller.toggleCam()}>
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

function StatsOverlay({
  stats,
  tryingDirect,
  forceRelay,
  onForceRelay,
}: {
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
    <div className="stats">
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
