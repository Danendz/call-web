import {
  Activity,
  Camera,
  Captions,
  MessageSquare,
  Mic,
  MicOff,
  PhoneOff,
  RectangleVertical,
  RotateCw,
  ScreenShare,
  ScreenShareOff,
  SwitchCamera,
  Video as VideoIcon,
  VideoOff,
} from "lucide-react";
import { useState } from "react";
import { useT } from "../ui/i18n";
import { IconButton } from "../ui/IconButton";
import type { CallController, CallState } from "./controller";
import { MoreMenu, type MenuItem } from "./MoreMenu";

/** Floating call controls. Compact drops speech-to-chat into the ⋯ menu to fit above the chat on phones. */
export function ControlPill({
  controller,
  state,
  compact,
  chatOpen,
  unread,
  showStats,
  canRotate,
  onRotate,
  canScreenshot,
  onScreenshot,
  onToggleChat,
  onToggleStats,
  onMenuChange,
  onHover,
}: {
  controller: CallController;
  state: CallState;
  compact: boolean;
  chatOpen: boolean;
  unread: number;
  showStats: boolean;
  canRotate: boolean;
  onRotate: () => void;
  canScreenshot: boolean;
  onScreenshot: () => void;
  onToggleChat: () => void;
  onToggleStats: () => void;
  onMenuChange: (open: boolean) => void;
  onHover: (hovering: boolean) => void;
}) {
  const t = useT();
  const [menuOpen, setMenuOpen] = useState(false);
  const setMenu = (open: boolean) => {
    setMenuOpen(open);
    onMenuChange(open);
  };

  const canShare = typeof navigator.mediaDevices?.getDisplayMedia === "function";
  const micGranted = state.micAccess === "granted";
  const camGranted = state.camAccess === "granted";
  const micOn = micGranted && state.micOn;
  const camOn = camGranted && state.camOn;

  const transcribe: Omit<MenuItem, "icon"> = {
    label: t.speechToChat,
    checked: state.transcribeOn,
    disabled: !micGranted,
    onSelect: () => controller.toggleTranscribe(),
  };
  // Two cameras is a phone's front and back: one flip item. More (a laptop with USB cameras) are listed to pick from.
  const cameras: MenuItem[] =
    !camGranted || state.cams.length < 2
      ? []
      : state.cams.length === 2
        ? [{ label: t.switchCamera, icon: <SwitchCamera size={18} />, onSelect: () => controller.nextCamera() }]
        : state.cams.map((cam) => ({
            label: cam.label,
            icon: <VideoIcon size={18} />,
            checked: cam.id === state.camDeviceId,
            onSelect: () => void controller.selectDevice("video", cam.id),
          }));
  const items: MenuItem[] = [
    ...cameras,
    ...(compact ? [{ ...transcribe, icon: <Captions size={18} /> }] : []),
    ...(canShare
      ? [
          {
            label: state.sharing ? t.stopSharing : t.shareScreen,
            icon: state.sharing ? <ScreenShareOff size={18} /> : <ScreenShare size={18} />,
            disabled: state.phase !== "in-call",
            onSelect: () => controller.toggleScreenShare(),
          },
        ]
      : []),
    {
      label: t.portraitVideo,
      icon: <RectangleVertical size={18} />,
      checked: state.orientation === "portrait",
      disabled: !camGranted,
      onSelect: () => controller.setOrientation(state.orientation === "portrait" ? "landscape" : "portrait"),
    },
    { label: t.rotateVideo, icon: <RotateCw size={18} />, disabled: !canRotate, onSelect: onRotate },
    { label: t.screenshot, icon: <Camera size={18} />, disabled: !canScreenshot, onSelect: onScreenshot },
    { label: t.stats, icon: <Activity size={18} />, checked: showStats, onSelect: onToggleStats },
  ];

  return (
    <div className="pill" onPointerEnter={() => onHover(true)} onPointerLeave={() => onHover(false)}>
      <IconButton
        label={micOn ? t.micOff : t.micOn}
        variant={micGranted && !micOn ? "off" : null}
        disabled={!micGranted}
        onClick={() => controller.toggleMic()}
      >
        {micOn ? <Mic size={20} /> : <MicOff size={20} />}
      </IconButton>
      <IconButton
        label={camOn ? t.camOff : t.camOn}
        variant={camGranted && !camOn ? "off" : null}
        disabled={!camGranted}
        onClick={() => controller.toggleCam()}
      >
        {camOn ? <VideoIcon size={20} /> : <VideoOff size={20} />}
      </IconButton>
      {!compact && (
        <IconButton
          label={transcribe.label}
          variant={state.transcribeOn ? "active" : null}
          pressed={state.transcribeOn}
          disabled={transcribe.disabled}
          onClick={transcribe.onSelect}
        >
          <Captions size={20} />
        </IconButton>
      )}
      <IconButton
        label={t.chat}
        variant={chatOpen ? "active" : null}
        pressed={chatOpen}
        badge={chatOpen ? 0 : unread}
        onClick={onToggleChat}
      >
        <MessageSquare size={20} />
      </IconButton>
      <MoreMenu items={items} open={menuOpen} onOpenChange={setMenu} />
      <IconButton label={t.hangUp} variant="danger" className="hang-up" onClick={() => controller.hangUp()}>
        <PhoneOff size={20} />
      </IconButton>
    </div>
  );
}
