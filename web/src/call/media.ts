/** What happened when we asked for a device: denied = blocked by the user/browser, busy = another app holds it. */
export type MediaAccess = "pending" | "granted" | "denied" | "missing" | "busy";
export type MediaKind = "audio" | "video";
/** Shape of the camera video we send; the other side's view fits whatever arrives. */
export type Orientation = "landscape" | "portrait";

export interface DeviceOption {
  id: string;
  label: string;
}

const AUDIO: MediaTrackConstraints = { echoCancellation: true, noiseSuppression: true, autoGainControl: true };

export function videoConstraints(orientation: Orientation): MediaTrackConstraints {
  const [width, height] = orientation === "portrait" ? [720, 1280] : [1280, 720];
  return { width: { ideal: width }, height: { ideal: height }, aspectRatio: { ideal: width / height }, frameRate: { ideal: 30, max: 30 } };
}

function constraintsFor(kind: MediaKind, deviceId: string | null): MediaTrackConstraints {
  const base = kind === "audio" ? AUDIO : videoConstraints(loadOrientation());
  if (deviceId) return { ...base, deviceId: { ideal: deviceId } }; // ideal: falls back if the device is gone
  return kind === "video" ? { ...base, facingMode: "user" } : base;
}

export async function getLocalMedia(
  micId: string | null,
  camId: string | null,
): Promise<{ stream: MediaStream; audio: MediaAccess; video: MediaAccess }> {
  if (!navigator.mediaDevices?.getUserMedia) return { stream: new MediaStream(), audio: "missing", video: "missing" };
  try {
    // One request, so the browser shows a single permission prompt.
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: constraintsFor("audio", micId),
      video: constraintsFor("video", camId),
    });
    return { stream, audio: "granted", video: "granted" };
  } catch {
    // No camera, camera busy or one of them denied: ask separately to find out which, and keep the other.
    const audio = await getTrack("audio", micId);
    const video = await getTrack("video", camId);
    const tracks = [audio.track, video.track].filter((t): t is MediaStreamTrack => t !== null);
    return { stream: new MediaStream(tracks), audio: audio.access, video: video.access };
  }
}

export async function getTrack(kind: MediaKind, deviceId: string | null): Promise<{ track: MediaStreamTrack | null; access: MediaAccess }> {
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ [kind]: constraintsFor(kind, deviceId) });
    return { track: stream.getTracks()[0], access: "granted" };
  } catch (err) {
    const name = (err as DOMException).name;
    if (name === "NotAllowedError" || name === "SecurityError") return { track: null, access: "denied" };
    if (name === "NotFoundError" || name === "OverconstrainedError") return { track: null, access: "missing" };
    return { track: null, access: "busy" };
  }
}

export function selectedDevices(stream: MediaStream): { micDeviceId: string | null; camDeviceId: string | null } {
  return {
    micDeviceId: stream.getAudioTracks()[0]?.getSettings().deviceId ?? null,
    camDeviceId: stream.getVideoTracks()[0]?.getSettings().deviceId ?? null,
  };
}

/** Your own preview is shown mirrored like a selfie, except for a phone's back camera, which shows the world. */
export function isRearCamera(stream: MediaStream): boolean {
  return stream.getVideoTracks()[0]?.getSettings().facingMode === "environment";
}

export function loadDevicePref(kind: MediaKind): string | null {
  try {
    return localStorage.getItem(`device.${kind}`);
  } catch {
    return null;
  }
}

export function saveDevicePref(kind: MediaKind, deviceId: string): void {
  try {
    localStorage.setItem(`device.${kind}`, deviceId);
  } catch {
    // storage blocked: the choice just isn't remembered
  }
}

export function loadOrientation(): Orientation {
  try {
    return localStorage.getItem("orientation") === "portrait" ? "portrait" : "landscape";
  } catch {
    return "landscape";
  }
}

export function saveOrientation(orientation: Orientation): void {
  try {
    localStorage.setItem("orientation", orientation);
  } catch {
    // storage blocked: the choice just isn't remembered
  }
}

/**
 * Asks for tab or system audio too; Chrome shows a "share audio" tick, other browsers ignore it or refuse audio, in
 * which case it asks again for video alone. The call's own tab is left out of the picker: sharing it would echo the
 * other person back to them.
 */
export async function getDisplayMedia(): Promise<MediaStream> {
  // As smooth as the browser can capture: a playing video or a game needs it, and short of bandwidth the encoder
  // lowers the frame rate itself
  const video = { frameRate: { ideal: 60 } };
  // Chrome-only fields, not in TypeScript's DOM types yet
  const withAudio: DisplayMediaStreamOptions & { selfBrowserSurface: string; systemAudio: string } = {
    video,
    audio: true,
    selfBrowserSurface: "exclude",
    systemAudio: "include",
  };
  try {
    return await navigator.mediaDevices.getDisplayMedia(withAudio);
  } catch (err) {
    if ((err as DOMException).name !== "TypeError" && (err as DOMException).name !== "NotSupportedError") throw err;
    return navigator.mediaDevices.getDisplayMedia({ video, audio: false });
  }
}
