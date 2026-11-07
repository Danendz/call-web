import { VideoOff } from "lucide-react";
import { useState, type CSSProperties, type ReactNode } from "react";
import { useT } from "../ui/i18n";
import { Video } from "../ui/Video";

/**
 * The two videos: one fills the stage, the other is the small tile. Swapping only changes their classes, so each
 * keeps its <video> element and CSS animates it between the two spots.
 */
export function Feeds({
  localStream,
  remoteStream,
  camOn,
  swapped,
  remoteRotation,
  onSwap,
}: {
  localStream: MediaStream | null;
  remoteStream: MediaStream | null;
  camOn: boolean;
  swapped: boolean;
  /** Clockwise degrees, a multiple of 90: fixes a picture that arrives sideways. Only on this screen. */
  remoteRotation: number;
  onSwap: () => void;
}) {
  const [localAspect, setLocalAspect] = useState(16 / 9);
  const [remoteAspect, setRemoteAspect] = useState(16 / 9);
  // Alone in the room, your own camera fills the screen
  const localMain = !remoteStream || swapped;

  return (
    <>
      {remoteStream && (
        <Feed main={!localMain} aspect={remoteRotation % 180 ? 1 / remoteAspect : remoteAspect} onSwap={onSwap}>
          <Video stream={remoteStream} className="feed-video" rotation={remoteRotation} onAspect={setRemoteAspect} />
        </Feed>
      )}
      {localStream && (
        <Feed main={localMain} aspect={localAspect} onSwap={remoteStream ? onSwap : null}>
          <Video stream={localStream} className="feed-video" muted mirror onAspect={setLocalAspect} />
          <div className={`feed-off ${camOn ? "" : "shown"}`} aria-hidden={camOn}>
            <VideoOff size={localMain ? 36 : 20} />
          </div>
        </Feed>
      )}
    </>
  );
}

function Feed({
  main,
  aspect,
  onSwap,
  children,
}: {
  main: boolean;
  aspect: number;
  onSwap: (() => void) | null;
  children: ReactNode;
}) {
  const t = useT();
  // The tile's long edge is fixed in CSS (--pip-long); the short one follows the frame shape.
  const size =
    aspect >= 1
      ? { "--pip-w": "var(--pip-long)", "--pip-h": `calc(var(--pip-long) / ${aspect.toFixed(4)})` }
      : { "--pip-w": `calc(var(--pip-long) * ${aspect.toFixed(4)})`, "--pip-h": "var(--pip-long)" };
  const swappable = !main && onSwap;
  return (
    <div
      className={`feed ${main ? "main" : "pip"}`}
      style={size as CSSProperties}
      role={swappable ? "button" : undefined}
      tabIndex={swappable ? 0 : undefined}
      aria-label={swappable ? t.swapVideos : undefined}
      title={swappable ? t.swapVideos : undefined}
      onClick={swappable ? onSwap : undefined}
      onKeyDown={swappable ? (e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), onSwap()) : undefined}
    >
      {children}
    </div>
  );
}
