import { useEffect, useState } from "react";
import { ChatOverlay } from "../chat/ChatOverlay";
import { useReactionRuns } from "../chat/useReactionRuns";
import { SettingsSheet } from "../settings/SettingsSheet";
import { useShowStats } from "../settings/stats";
import { useSubtitles } from "../settings/subtitles";
import { TranslationOffer } from "../translate/TranslationOffer";
import { CopyLink } from "../ui/CopyLink";
import { useT, type Strings } from "../ui/i18n";
import { useSoundBlocked } from "../ui/Video";
import "./call.css";
import { CallTimer } from "./CallTimer";
import { ControlPill } from "./ControlPill";
import type { CallController, CallState } from "./controller";
import { Fade } from "./Fade";
import { arrangeFeeds, DEFAULT_CORNERS, Feeds, type FeedKey } from "./Feeds";
import { isRearCamera } from "./media";
import { Reactions } from "./reactions/Reactions";
import { captureCall, saveImage, screenshotName, type Shot } from "./screenshot";
import { StatsOverlay } from "./StatsOverlay";
import { useAutoHide } from "./useAutoHide";
import { useCorners, type Corner } from "./useDragCorner";
import { SHEET_QUERY, useMediaQuery } from "../ui/useMediaQuery";
import { useVisualViewport } from "./useVisualViewport";

export function CallView({ controller, state, roomUrl }: { controller: CallController; state: CallState; roomUrl: string }) {
  const t = useT();
  const [chatOpen, setChatOpen] = useState(true);
  const showStats = useShowStats();
  const subtitles = useSubtitles();
  const soundBlocked = useSoundBlocked();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [seenCount, setSeenCount] = useState(0);
  // The feed the user tapped onto the stage; null for the default (their screen, else them)
  const [picked, setPicked] = useState<FeedKey | null>(null);
  const { corners, setCorner } = useCorners(DEFAULT_CORNERS);
  const [remoteRotation, setRemoteRotation] = useState(0);
  const [menuOpen, setMenuOpen] = useState(false);
  const [hoveringPill, setHoveringPill] = useState(false);
  const [flash, setFlash] = useState(0);
  const narrow = useMediaQuery(SHEET_QUERY);
  const finePointer = useMediaQuery("(pointer: fine)");
  useVisualViewport();

  const reactions = useReactionRuns(controller.onReaction, state.chat.length);

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
  const { idle, wake, sleep } = useAutoHide(connected && !menuOpen && !settingsOpen && !hoveringPill);

  const onPointerDown = (e: React.PointerEvent) => {
    // On touch, tapping the bare video toggles the controls, like in phone call apps
    const onVideo = !(e.target as Element).closest(".pill, .pip, .stats, .chat, .sheet, .sheet-dim");
    if (e.pointerType !== "mouse" && onVideo && !idle) sleep();
    else wake();
  };

  // Tiles leave the corners the chat covers for the top ones, keeping the corner they were put in for when it's hidden
  const tileCorners = chatOpen ? clearOfChat(corners, narrow) : corners;

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
    setCorner(layout.main, tileCorners[key]);
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
          return shot ? [{ ...shot, corner: tileCorners[key] }] : [];
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
      className={`call ${idle ? "idle" : ""} ${chatOpen ? "chat-open" : ""}`}
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
          corners={tileCorners}
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

        <Reactions controller={controller} />

        {flash > 0 && <div key={flash} className="shot-flash" aria-hidden />}

        <div className="notices">
          {connected && state.callStartedAt !== null && <CallTimer startedAt={state.callStartedAt} />}
          <Fade className="notice" text={banner} />
          <Fade className="notice" text={soundBlocked ? t.tapToListen : null} />
          <Fade className="notice muted" text={state.peerScreenshot ? t.peerScreenshot : null} />
          <Fade className="notice muted" text={state.transcribeOn && state.speechNotice ? t.speechUnavailable : null} />
          {connected && <TranslationOffer lang={state.lang} />}
        </div>
        <ChatOverlay
          open={chatOpen}
          messages={state.chat}
          reactions={reactions}
          live={state.live}
          myId={state.myId}
          lang={state.lang}
          liveOn={subtitles}
          focusOnOpen={finePointer}
          onSend={(t) => controller.sendChat(t)}
        />
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
          canReact={connected}
          canScreenshot={canScreenshot}
          onScreenshot={takeScreenshot}
          onToggleChat={() => setChatOpen((v) => !v)}
          onOpenSettings={() => setSettingsOpen(true)}
          onMenuChange={setMenuOpen}
          onHover={setHoveringPill}
        />
      </div>

      <SettingsSheet open={settingsOpen} onClose={() => setSettingsOpen(false)} lang={state.lang} onLang={(l) => controller.setLang(l)} />
    </div>
  );
}

/** The chat covers the bottom-left corner, and on phones, where it spans the width, the bottom-right too. */
function clearOfChat<K extends string>(corners: Record<K, Corner>, narrow: boolean): Record<K, Corner> {
  const moved: Partial<Record<Corner, Corner>> = narrow ? { bl: "tl", br: "tr" } : { bl: "tl" };
  const out = { ...corners };
  for (const key in out) out[key] = moved[out[key]] ?? out[key];
  return out;
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
