import { VideoOff, VolumeOff } from "lucide-react";
import { useState, type CSSProperties, type ReactNode } from "react";
import { useT } from "../ui/i18n";
import { Video } from "../ui/Video";
import { isRearCamera } from "./media";
import { useDragCorner, type Corner } from "./useDragCorner";

/** Your camera, the other person's camera, and their shared screen. */
export type FeedKey = "self" | "peer" | "screen";

export const DEFAULT_CORNERS: Record<FeedKey, Corner> = { self: "br", peer: "tr", screen: "tl" };

export interface FeedLayout {
  main: FeedKey;
  /** The small tiles; the first one in a corner sits in it, a second stacks next to it. */
  tiles: FeedKey[];
  /** Kept playing (its audio) but not shown: the other person's turned-off camera while their screen is up. */
  hidden: FeedKey | null;
}

/** Picks what fills the stage: the feed the user tapped, else their screen, else them, else you (alone). */
export function arrangeFeeds({
  self,
  peer,
  screen,
  peerCamOn,
  picked,
}: {
  self: boolean;
  peer: boolean;
  screen: boolean;
  peerCamOn: boolean;
  picked: FeedKey | null;
}): FeedLayout {
  const hidden = peer && screen && !peerCamOn ? "peer" : null;
  const shown = (["self", "peer", "screen"] as const).filter((k) => ({ self, peer, screen })[k] && k !== hidden);
  const main = picked && shown.includes(picked) ? picked : screen ? "screen" : peer && !hidden ? "peer" : "self";
  return { main, tiles: shown.filter((k) => k !== main), hidden };
}

/**
 * Up to three videos: one fills the stage, the others are small tiles. Tapping a tile puts it on the stage; dragging
 * moves it to another corner. Each feed keeps its <video> element whatever its role, so CSS animates it between spots.
 */
export function Feeds({
  localStream,
  remoteStream,
  remoteScreen,
  camOn,
  remoteCamOn,
  speakerOff,
  speakerId,
  remoteSpeakerOff,
  layout,
  corners,
  remoteRotation,
  onPick,
  onMove,
}: {
  localStream: MediaStream | null;
  remoteStream: MediaStream | null;
  remoteScreen: MediaStream | null;
  camOn: boolean;
  remoteCamOn: boolean;
  /** You muted the other person: their camera and screen play silently. */
  speakerOff: boolean;
  /** The speaker their sound plays on; null for the default. */
  speakerId: string | null;
  /** The other person muted you, shown on their video so you know they can't hear you. */
  remoteSpeakerOff: boolean;
  layout: FeedLayout;
  corners: Record<FeedKey, Corner>;
  /** Clockwise degrees, a multiple of 90: fixes a picture that arrives sideways. Only on this screen. */
  remoteRotation: number;
  onPick: (key: FeedKey) => void;
  onMove: (key: FeedKey, corner: Corner) => void;
}) {
  const t = useT();
  const sinkId = speakerId ?? "";
  const [aspects, setAspects] = useState<Record<FeedKey, number>>({ self: 16 / 9, peer: 16 / 9, screen: 16 / 9 });
  const onAspect = (key: FeedKey) => (aspect: number) => setAspects((prev) => ({ ...prev, [key]: aspect }));
  const sizes = {
    self: tileSize(aspects.self),
    peer: tileSize(remoteRotation % 180 ? 1 / aspects.peer : aspects.peer),
    screen: tileSize(aspects.screen),
  };

  const feed = (key: FeedKey, children: ReactNode) => {
    // A tile sharing its corner with an earlier one moves past it
    const before = layout.tiles.slice(0, Math.max(0, layout.tiles.indexOf(key))).find((k) => corners[k] === corners[key]);
    return (
      <Feed
        role={key === layout.main ? "main" : key === layout.hidden ? "hidden" : "pip"}
        corner={corners[key]}
        size={sizes[key]}
        stack={before ? `calc(${sizes[before].h} + var(--pip-gap))` : null}
        onPick={() => onPick(key)}
        onMove={(corner) => onMove(key, corner)}
      >
        {children}
      </Feed>
    );
  };

  return (
    <>
      {remoteStream &&
        feed(
          "peer",
          <>
            <Video
              stream={remoteStream}
              className="feed-video"
              muted={speakerOff}
              sinkId={sinkId}
              rotation={remoteRotation}
              onAspect={onAspect("peer")}
            />
            <CamOff shown={!remoteCamOn} large={layout.main === "peer"} />
            {remoteSpeakerOff && (
              <span className="feed-mark" title={t.peerSpeakerOff} aria-label={t.peerSpeakerOff}>
                <VolumeOff size={16} />
              </span>
            )}
          </>,
        )}
      {remoteScreen &&
        feed(
          "screen",
          <Video stream={remoteScreen} className="feed-video" muted={speakerOff} sinkId={sinkId} onAspect={onAspect("screen")} />,
        )}
      {localStream &&
        feed(
          "self",
          <>
            <Video
              stream={localStream}
              className="feed-video"
              muted
              mirror={!isRearCamera(localStream)}
              onAspect={onAspect("self")}
            />
            <CamOff shown={!camOn} large={layout.main === "self"} />
          </>,
        )}
    </>
  );
}

function CamOff({ shown, large }: { shown: boolean; large: boolean }) {
  return (
    <div className={`feed-off ${shown ? "shown" : ""}`} aria-hidden={!shown}>
      <VideoOff size={large ? 36 : 20} />
    </div>
  );
}

/** The tile's long edge is fixed in CSS (--pip-long); the short one follows the frame shape. */
function tileSize(aspect: number): { w: string; h: string } {
  return aspect >= 1
    ? { w: "var(--pip-long)", h: `calc(var(--pip-long) / ${aspect.toFixed(4)})` }
    : { w: `calc(var(--pip-long) * ${aspect.toFixed(4)})`, h: "var(--pip-long)" };
}

function Feed({
  role,
  corner,
  size,
  stack,
  onPick,
  onMove,
  children,
}: {
  role: "main" | "pip" | "hidden";
  corner: Corner;
  size: { w: string; h: string };
  /** How far a second tile in the same corner moves along from it */
  stack: string | null;
  onPick: () => void;
  onMove: (corner: Corner) => void;
  children: ReactNode;
}) {
  const t = useT();
  const drag = useDragCorner(onMove);
  const pip = role === "pip";
  const style = { "--pip-w": size.w, "--pip-h": size.h, ...(stack && { "--pip-stack": stack }), ...drag.style };
  return (
    <div
      className={`feed ${role === "main" ? "main" : "pip"} ${role === "hidden" ? "gone" : ""} ${drag.dragging ? "dragging" : ""}`}
      data-corner={corner}
      style={style as CSSProperties}
      role={pip ? "button" : undefined}
      tabIndex={pip ? 0 : undefined}
      aria-hidden={role === "hidden" || undefined}
      aria-label={pip ? t.swapVideos : undefined}
      title={pip ? t.swapVideos : undefined}
      {...(pip ? drag.handlers : {})}
      onClick={pip ? () => !drag.wasDragged() && onPick() : undefined}
      onKeyDown={pip ? (e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), onPick()) : undefined}
    >
      {children}
    </div>
  );
}
