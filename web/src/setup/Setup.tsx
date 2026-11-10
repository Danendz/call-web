import { Languages, Mic, MicOff, RectangleHorizontal, RectangleVertical, Settings, Video as VideoIcon, VideoOff, Volume2 } from "lucide-react";
import { useEffect, useState, type CSSProperties } from "react";
import { LANGS, type Lang } from "../../../shared/protocol";
import { LANG_NAMES } from "../call/speech/lang";
import type { CallController, CallState } from "../call/controller";
import { isRearCamera } from "../call/media";
import { SettingsSheet } from "../settings/SettingsSheet";
import { useT } from "../ui/i18n";
import { IconButton } from "../ui/IconButton";
import { Video } from "../ui/Video";
import { DeviceRow } from "./DeviceRow";
import { DownloadApp } from "./DownloadApp";
import { MicLevel } from "./MicLevel";
import "./setup.css";

/** Pre-join screen: camera preview, mic level, device pickers and permission help. */
export function Setup({ controller, state, onSignIn }: { controller: CallController; state: CallState; onSignIn?: () => void }) {
  const t = useT();
  useEffect(() => {
    void controller.preview();
  }, [controller]);

  const portrait = state.orientation === "portrait";
  const [aspect, setAspect] = useState<number | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  // Until the camera reports its frame size, assume it gives the shape we asked for
  useEffect(() => setAspect(null), [state.orientation]);

  const micGranted = state.micAccess === "granted";
  const camGranted = state.camAccess === "granted";
  const pending = state.micAccess === "pending" || state.camAccess === "pending";
  const blocked = state.micAccess === "denied" || state.camAccess === "denied";
  const showVideo = state.localStream && camGranted && state.camOn;
  const joinLabel =
    pending ? t.waitingPermission
    : !micGranted && !camGranted ? t.joinWithoutMedia
    : t.joinCall;

  return (
    <div className="setup">
      <div className="preview" style={{ "--ar": aspect ?? (portrait ? 9 / 16 : 16 / 9) } as CSSProperties}>
        {showVideo ? (
          <Video stream={state.localStream!} className="preview-video" muted mirror={!isRearCamera(state.localStream!)} onAspect={setAspect} />
        ) : (
          <div className="preview-off">
            <VideoOff size={28} />
            <span>{camGranted ? t.cameraOff : t.deviceStatus(t.camera, t.access[state.camAccess])}</span>
          </div>
        )}
        <div className="preview-controls">
          <IconButton
            label={state.micOn ? t.micOff : t.micOn}
            variant={micGranted && !state.micOn ? "off" : null}
            disabled={!micGranted}
            onClick={() => controller.toggleMic()}
          >
            {micGranted && state.micOn && state.localStream && <MicLevel stream={state.localStream} />}
            {micGranted && state.micOn ? <Mic size={20} /> : <MicOff size={20} />}
          </IconButton>
          <IconButton
            label={state.camOn ? t.camOff : t.camOn}
            variant={camGranted && !state.camOn ? "off" : null}
            disabled={!camGranted}
            onClick={() => controller.toggleCam()}
          >
            {camGranted && state.camOn ? <VideoIcon size={20} /> : <VideoOff size={20} />}
          </IconButton>
          <IconButton
            label={portrait ? t.sendLandscape : t.sendPortrait}
            disabled={!camGranted}
            onClick={() => controller.setOrientation(portrait ? "landscape" : "portrait")}
          >
            {portrait ? <RectangleVertical size={20} /> : <RectangleHorizontal size={20} />}
          </IconButton>
        </div>
      </div>

      <div className="setup-panel">
        <div className="panel-head">
          <h1>{t.readyToJoin}</h1>
          <div className="panel-actions">
            {onSignIn && <button onClick={onSignIn}>{t.signIn}</button>}
            <DownloadApp />
            <IconButton label={t.settings} onClick={() => setSettingsOpen(true)}>
              <Settings size={18} />
            </IconButton>
          </div>
        </div>

        <div className="devices">
          <DeviceRow
            name={t.microphone}
            icon={<Mic size={18} />}
            access={state.micAccess}
            devices={state.mics}
            selected={state.micDeviceId}
            opening={state.switching.audio}
            onSelect={(id) => controller.selectDevice("audio", id)}
          />
          {state.speakers.length > 0 && (
            <DeviceRow
              name={t.speaker}
              icon={<Volume2 size={18} />}
              access="granted"
              devices={state.speakers}
              selected={state.speakerDeviceId ?? state.speakers[0].id}
              onSelect={(id) => controller.selectSpeaker(id)}
            />
          )}
          <DeviceRow
            name={t.camera}
            icon={<VideoIcon size={18} />}
            access={state.camAccess}
            devices={state.cams}
            selected={state.camDeviceId}
            opening={state.switching.video}
            onSelect={(id) => controller.selectDevice("video", id)}
          />
          <label className="device-row">
            <span className="row-icon" aria-hidden>
              <Languages size={18} />
            </span>
            <span className="row-label">{t.iSpeak}</span>
            <select value={state.lang} onChange={(e) => controller.setLang(e.target.value as Lang)}>
              {LANGS.map((l) => (
                <option key={l} value={l}>
                  {LANG_NAMES[l]}
                </option>
              ))}
            </select>
          </label>
        </div>

        {blocked && (
          <div className="permission-help">
            <p>{t.permissionHelp}</p>
            <button onClick={() => controller.preview()}>{t.retry}</button>
          </div>
        )}

        <button className="primary join" disabled={pending} onClick={() => controller.start()}>
          {joinLabel}
        </button>
      </div>

      <SettingsSheet open={settingsOpen} onClose={() => setSettingsOpen(false)} lang={state.lang} onLang={(l) => controller.setLang(l)} />
    </div>
  );
}
