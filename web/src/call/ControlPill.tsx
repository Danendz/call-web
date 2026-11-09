import {
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
  Settings,
  Video as VideoIcon,
  VideoOff,
  VolumeOff,
} from "lucide-react";
import { useState } from "react";
import { useT } from "../ui/i18n";
import { IconButton } from "../ui/IconButton";
import { Spinner } from "../ui/Spinner";
import type { CallController, CallState } from "./controller";
import { DeviceMenu, type MenuSection } from "./DeviceMenu";
import type { DeviceOption } from "./media";
import { MoreMenu, type MenuItem } from "./MoreMenu";

type OpenMenu = "mic" | "cam" | "more" | null;

/** Floating call controls. The mic and camera chevrons hold device choices; the rest goes in ⋯. */
export function ControlPill({
  controller,
  state,
  chatOpen,
  unread,
  canRotate,
  onRotate,
  canScreenshot,
  onScreenshot,
  onToggleChat,
  onOpenSettings,
  onMenuChange,
  onHover,
}: {
  controller: CallController;
  state: CallState;
  chatOpen: boolean;
  unread: number;
  canRotate: boolean;
  onRotate: () => void;
  canScreenshot: boolean;
  onScreenshot: () => void;
  onToggleChat: () => void;
  onOpenSettings: () => void;
  onMenuChange: (open: boolean) => void;
  onHover: (hovering: boolean) => void;
}) {
  const t = useT();
  const [openMenu, setOpenMenu] = useState<OpenMenu>(null);
  const menu = (which: Exclude<OpenMenu, null>) => ({
    open: openMenu === which,
    onOpenChange: (open: boolean) => {
      const next = open ? which : openMenu === which ? null : openMenu;
      setOpenMenu(next);
      onMenuChange(next !== null);
    },
  });

  const canShare = typeof navigator.mediaDevices?.getDisplayMedia === "function";
  const micGranted = state.micAccess === "granted";
  const camGranted = state.camAccess === "granted";
  const micOn = micGranted && state.micOn;
  const camOn = camGranted && state.camOn;

  // While one opens, it gets the spinner and the check mark leaves the old one
  const devices = (list: DeviceOption[], current: string | null, opening: string | null, onPick: (id: string) => void): MenuItem[] =>
    list.map((d) => ({
      id: d.id,
      label: d.label,
      icon: null,
      checked: !opening && d.id === current,
      busy: d.id === opening,
      onSelect: () => onPick(d.id),
    }));
  const micSections: MenuSection[] = [
    ...(micGranted
      ? [{ title: t.microphone, items: devices(state.mics, state.micDeviceId, state.switching.audio, (id) => void controller.selectDevice("audio", id)) }]
      : []),
    {
      title: t.speaker,
      toggle: { label: t.muteSpeaker, icon: <VolumeOff size={14} />, on: state.speakerOff, onToggle: () => controller.toggleSpeaker() },
      // Nothing picked plays on the default, which browsers list first
      items: devices(state.speakers, state.speakerDeviceId ?? state.speakers[0]?.id ?? null, null, (id) => controller.selectSpeaker(id)),
    },
  ];
  const speech: MenuItem = {
    label: t.speechToChat,
    icon: <Captions size={18} />,
    checked: state.transcribeOn,
    disabled: !micGranted,
    onSelect: () => controller.toggleTranscribe(),
  };

  const items: MenuItem[] = [
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
    { label: t.settings, icon: <Settings size={18} />, onSelect: onOpenSettings },
  ];

  const camButton = (
    <IconButton
      label={camOn ? t.camOff : t.camOn}
      variant={camGranted && !camOn ? "off" : null}
      disabled={!camGranted}
      mark={state.switching.video ? <Spinner size={11} /> : null}
      onClick={() => controller.toggleCam()}
    >
      {camOn ? <VideoIcon size={20} /> : <VideoOff size={20} />}
    </IconButton>
  );

  return (
    <div className="pill" onPointerEnter={() => onHover(true)} onPointerLeave={() => onHover(false)}>
      <DeviceMenu label={t.micOptions} sections={micSections} footer={[speech]} {...menu("mic")}>
        <IconButton
          label={micOn ? t.micOff : t.micOn}
          variant={micGranted && !micOn ? "off" : null}
          disabled={!micGranted}
          mark={
            state.switching.audio ? <Spinner size={11} />
            : state.speakerOff ? <VolumeOff size={11} />
            : state.transcribeOn ? <Captions size={11} />
            : null
          }
          onClick={() => controller.toggleMic()}
        >
          {micOn ? <Mic size={20} /> : <MicOff size={20} />}
        </IconButton>
      </DeviceMenu>
      {camGranted && state.cams.length > 1 ? (
        <DeviceMenu
          label={t.camOptions}
          sections={[{ title: t.camera, items: devices(state.cams, state.camDeviceId, state.switching.video, (id) => void controller.selectDevice("video", id)) }]}
          {...menu("cam")}
        >
          {camButton}
        </DeviceMenu>
      ) : (
        camButton
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
      <MoreMenu items={items} {...menu("more")} />
      <IconButton label={t.hangUp} variant="danger" className="hang-up" onClick={() => controller.hangUp()}>
        <PhoneOff size={20} />
      </IconButton>
    </div>
  );
}
