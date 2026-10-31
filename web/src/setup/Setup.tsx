import { useEffect } from "react";
import { LANGS, type Lang } from "../../../shared/protocol";
import { LANG_NAMES } from "../call/speech/lang";
import type { CallController, CallState } from "../call/controller";
import { CopyLink } from "../ui/CopyLink";
import { Video } from "../ui/Video";
import { ACCESS_TEXT, DeviceRow } from "./DeviceRow";
import { MicLevel } from "./MicLevel";

/** Pre-join screen: camera preview, mic level, device pickers and permission help. */
export function Setup({ controller, state, roomUrl }: { controller: CallController; state: CallState; roomUrl: string }) {
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

        <label className="device-row">
          <span className="row-label">I speak</span>
          <select value={state.lang} onChange={(e) => controller.setLang(e.target.value as Lang)}>
            {LANGS.map((l) => (
              <option key={l} value={l}>
                {LANG_NAMES[l]}
              </option>
            ))}
          </select>
        </label>

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
