import { useEffect, useState } from "react";
import { ChatPanel } from "../chat/ChatPanel";
import { CopyLink } from "../ui/CopyLink";
import { Video } from "../ui/Video";
import type { CallController, CallState } from "./controller";
import { StatsOverlay } from "./StatsOverlay";

export function CallView({ controller, state, roomUrl }: { controller: CallController; state: CallState; roomUrl: string }) {
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
