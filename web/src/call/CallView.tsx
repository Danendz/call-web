import { useEffect, useState } from "react";
import { ChatPanel } from "../chat/ChatPanel";
import { SettingsSheet } from "../settings/SettingsSheet";
import { useShowStats } from "../settings/stats";
import { useSubtitles } from "../settings/subtitles";
import { CopyLink } from "../ui/CopyLink";
import { useT, type Strings } from "../ui/i18n";
import "./call.css";
import { CallTimer } from "./CallTimer";
import { ControlPill } from "./ControlPill";
import type { CallController, CallState } from "./controller";
import type { SpeechNotice } from "./speech/transcriber";
import { Fade } from "./Fade";
import { arrangeFeeds, DEFAULT_CORNERS, Feeds, type FeedKey } from "./Feeds";
import { isRearCamera } from "./media";
import { captureCall, saveImage, screenshotName, type Shot } from "./screenshot";
import { StatsOverlay } from "./StatsOverlay";
import { Subtitles } from "./Subtitles";
import { useAutoHide } from "./useAutoHide";
import { useCorners } from "./useDragCorner";
import { STACKED_QUERY, useMediaQuery } from "../ui/useMediaQuery";
import { useVisualViewport } from "./useVisualViewport";

export function CallView({ controller, state, roomUrl }: { controller: CallController; state: CallState; roomUrl: string }) {
  const t = useT();
  const [chatOpen, setChatOpen] = useState(() => window.matchMedia("(min-width: 900px)").matches);
  const showStats = useShowStats();
  const subtitles = useSubtitles();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [seenCount, setSeenCount] = useState(0);
  // The feed the user tapped onto the stage; null for the default (their screen, else them)
  const [picked, setPicked] = useState<FeedKey | null>(null);
  const { corners, setCorner } = useCorners(DEFAULT_CORNERS);
  const [remoteRotation, setRemoteRotation] = useState(0);
  const [menuOpen, setMenuOpen] = useState(false);
  const [hoveringPill, setHoveringPill] = useState(false);
  const [flash, setFlash] = useState(0);
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
    setPicked(null);
    setRemoteRotation(0);
  }, [state.remoteStream]);

  // A share that starts or stops brings back the default: the screen on the stage while there is one
  const remoteSharing = !!state.remoteScreen;
  useEffect(() => setPicked(null), [remoteSharing]);

  const connected = state.phase === "in-call" && !!state.remoteStream;
  const remoteScreen = connected ? state.remoteScreen : null;
  const compact = stacked && chatOpen;
  const { idle, wake, sleep } = useAutoHide(connected && !menuOpen && !settingsOpen && !hoveringPill && !compact);

  const onPointerDown = (e: React.PointerEvent) => {
    // On touch, tapping the bare video toggles the controls, like in phone call apps
    const onVideo = !(e.target as Element).closest(".pill, .pip, .stats, .chat, .sheet, .sheet-dim");
    if (e.pointerType !== "mouse" && onVideo && !idle) sleep();
    else wake();
  };

  const banner = statusBanner(state, t);
  const camOn = state.camAccess === "granted" && state.camOn;
  const layout = arrangeFeeds({
    self: !!state.localStream,
    peer: connected,
    screen: !!remoteScreen,
    peerCamOn: state.remoteCamOn,
    picked,
  });
  // Tapping a tile swaps it with the stage: the video that was there takes the tile's corner
  const pick = (key: FeedKey) => {
    setCorner(layout.main, corners[key]);
    setPicked(key);
  };

  // What a screenshot draws for each feed, when there's a picture to draw
  const shots: Record<FeedKey, Shot | null> = {
    self:
      camOn && state.localStream?.getVideoTracks().length
        ? { stream: state.localStream, mirror: !isRearCamera(state.localStream) }
        : null,
    peer:
      connected && state.remoteCamOn && state.remoteStream?.getVideoTracks().length
        ? { stream: state.remoteStream, rotation: remoteRotation }
        : null,
    screen: remoteScreen ? { stream: remoteScreen } : null,
  };
  const canScreenshot = connected && !!shots[layout.main];

  const takeScreenshot = async () => {
    const main = shots[layout.main];
    if (!main) return;
    try {
      const blob = await captureCall({
        main,
        tiles: layout.tiles.flatMap((key) => {
          const shot = shots[key];
          return shot ? [{ ...shot, corner: corners[key] }] : [];
        }),
        elapsedMs: state.callStartedAt === null ? 0 : performance.now() - state.callStartedAt,
      });
      setFlash((n) => n + 1);
      controller.notifyScreenshot();
      await saveImage(blob, screenshotName());
    } catch (err) {
      console.warn("screenshot failed", err);
    }
  };

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
          remoteScreen={remoteScreen}
          camOn={camOn}
          remoteCamOn={state.remoteCamOn}
          speakerOff={state.speakerOff}
          speakerId={state.speakerDeviceId}
          remoteSpeakerOff={state.remoteSpeakerOff}
          layout={layout}
          corners={corners}
          remoteRotation={remoteRotation}
          onPick={pick}
          onMove={setCorner}
        />

        {!connected && (
          <div className="waiting">
            <div className="waiting-card">
              <p>{state.phase === "starting" ? t.starting : t.waitingForPeer}</p>
              <CopyLink url={roomUrl} />
            </div>
          </div>
        )}

        {flash > 0 && <div key={flash} className="shot-flash" aria-hidden />}

        <div className="notices">
          {connected && state.callStartedAt !== null && <CallTimer startedAt={state.callStartedAt} />}
          <Fade className="notice" text={banner} />
          <Fade className="notice muted" text={state.peerScreenshot ? t.peerScreenshot : null} />
          <Fade className="notice muted" text={state.transcribeOn && state.speechNotice ? speechNoticeText(state.speechNotice, t) : null} />
        </div>
        {subtitles && (
          <Subtitles messages={state.chat} live={state.live} myId={state.myId} lang={state.lang} chatOpen={chatOpen} />
        )}
        <StatsOverlay
          // While relay is forced the stats stay up: their switch is the way back
          open={showStats || state.forceRelay}
          stats={state.stats}
          tryingDirect={state.tryingDirect}
          forceRelay={state.forceRelay}
          onForceRelay={(v) => controller.setForceRelay(v)}
        />

        <ControlPill
          controller={controller}
          state={state}
          chatOpen={chatOpen}
          unread={unread}
          canRotate={connected}
          onRotate={() => setRemoteRotation((r) => (r + 90) % 360)}
          canScreenshot={canScreenshot}
          onScreenshot={takeScreenshot}
          onToggleChat={() => setChatOpen((v) => !v)}
          onOpenSettings={() => setSettingsOpen(true)}
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

      <SettingsSheet open={settingsOpen} onClose={() => setSettingsOpen(false)} lang={state.lang} onLang={(l) => controller.setLang(l)} />
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
