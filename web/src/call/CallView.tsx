import { useEffect, useState } from "react";
import { ChatPanel } from "../chat/ChatPanel";
import { CopyLink } from "../ui/CopyLink";
import { useT, type Strings } from "../ui/i18n";
import "./call.css";
import { ControlPill } from "./ControlPill";
import type { CallController, CallState } from "./controller";
import type { SpeechNotice } from "./speech/transcriber";
import { Fade } from "./Fade";
import { Feeds } from "./Feeds";
import { StatsOverlay } from "./StatsOverlay";
import { Subtitle } from "./Subtitle";
import { useAutoHide } from "./useAutoHide";
import { STACKED_QUERY, useMediaQuery } from "./useMediaQuery";
import { useVisualViewport } from "./useVisualViewport";

export function CallView({ controller, state, roomUrl }: { controller: CallController; state: CallState; roomUrl: string }) {
  const t = useT();
  const [chatOpen, setChatOpen] = useState(() => window.matchMedia("(min-width: 900px)").matches);
  const [showStats, setShowStats] = useState(state.forceRelay);
  const [seenCount, setSeenCount] = useState(0);
  const [swapped, setSwapped] = useState(false);
  const [remoteRotation, setRemoteRotation] = useState(0);
  const [menuOpen, setMenuOpen] = useState(false);
  const [hoveringPill, setHoveringPill] = useState(false);
  const stacked = useMediaQuery(STACKED_QUERY);
  const finePointer = useMediaQuery("(pointer: fine)");
  useVisualViewport();

  const othersCount = state.chat.filter((m) => m.from !== state.myId).length;
  useEffect(() => {
    if (chatOpen) setSeenCount(othersCount);
  }, [chatOpen, othersCount]);
  const unread = othersCount - seenCount;

  // A new peer starts in the usual layout: them big, you small, upright
  useEffect(() => {
    if (state.remoteStream) return;
    setSwapped(false);
    setRemoteRotation(0);
  }, [state.remoteStream]);

  const connected = state.phase === "in-call" && !!state.remoteStream;
  const compact = stacked && chatOpen;
  const { idle, wake, sleep } = useAutoHide(connected && !menuOpen && !hoveringPill && !compact);

  const onPointerDown = (e: React.PointerEvent) => {
    // On touch, tapping the bare video toggles the controls, like in phone call apps
    const onVideo = !(e.target as Element).closest(".pill, .pip, .stats, .chat");
    if (e.pointerType !== "mouse" && onVideo && !idle) sleep();
    else wake();
  };

  const banner = statusBanner(state, t);

  return (
    <div
      className={`call ${chatOpen ? "chat-open" : ""} ${idle ? "idle" : ""}`}
      onPointerDown={onPointerDown}
      onPointerMove={(e) => e.pointerType === "mouse" && wake()}
      onKeyDown={wake}
    >
      <div className="stage">
        <Feeds
          localStream={state.localStream}
          remoteStream={connected ? state.remoteStream : null}
          camOn={state.camAccess === "granted" && state.camOn}
          swapped={swapped}
          remoteRotation={remoteRotation}
          onSwap={() => setSwapped((v) => !v)}
        />

        {!connected && (
          <div className="waiting">
            <div className="waiting-card">
              <p>{state.phase === "starting" ? t.starting : t.waitingForPeer}</p>
              <CopyLink url={roomUrl} />
            </div>
          </div>
        )}

        <div className="notices">
          <Fade className="notice" text={banner} />
          <Fade className="notice muted" text={state.transcribeOn && state.speechNotice ? speechNoticeText(state.speechNotice, t) : null} />
        </div>
        <Subtitle messages={state.chat} myId={state.myId} lang={state.lang} chatOpen={chatOpen} />
        <StatsOverlay
          open={showStats}
          stats={state.stats}
          tryingDirect={state.tryingDirect}
          forceRelay={state.forceRelay}
          onForceRelay={(v) => controller.setForceRelay(v)}
        />

        <ControlPill
          controller={controller}
          state={state}
          compact={compact}
          chatOpen={chatOpen}
          unread={unread}
          showStats={showStats}
          canRotate={connected}
          onRotate={() => setRemoteRotation((r) => (r + 90) % 360)}
          onToggleChat={() => setChatOpen((v) => !v)}
          onToggleStats={() => setShowStats((v) => !v)}
          onMenuChange={setMenuOpen}
          onHover={setHoveringPill}
        />
      </div>

      <ChatPanel
        open={chatOpen}
        messages={state.chat}
        myId={state.myId}
        lang={state.lang}
        focusOnOpen={finePointer}
        onSend={(t) => controller.sendChat(t)}
        onClose={() => setChatOpen(false)}
      />
    </div>
  );
}

function statusBanner(state: CallState, t: Strings): string | null {
  if (state.socket === "reconnecting") return t.serverLost;
  if (state.phase !== "in-call") return null;
  switch (state.iceState) {
    case "new":
    case "checking":
      return t.connecting;
    case "disconnected":
      return t.unstable;
    case "failed":
      return t.failed;
    default:
      return null;
  }
}

function speechNoticeText(notice: SpeechNotice, t: Strings): string {
  if (notice.kind === "unavailable") return t.speechUnavailable;
  return t.speechPaused(new Date(notice.until).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }));
}
