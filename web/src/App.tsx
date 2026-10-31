import { useMemo, useSyncExternalStore } from "react";
import { ROOM_ID_RE } from "../../shared/protocol";
import { CallController } from "./call/controller";
import { CallView } from "./call/CallView";
import { Setup } from "./setup/Setup";
import { Centered } from "./ui/Centered";

const ROOM_ALPHABET = "abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";

function resolveRoomId(): string {
  const match = location.pathname.match(/^\/r\/([^/]+)\/?$/);
  if (match && ROOM_ID_RE.test(match[1])) return match[1];
  const bytes = crypto.getRandomValues(new Uint8Array(10));
  const id = Array.from(bytes, (b) => ROOM_ALPHABET[b % ROOM_ALPHABET.length]).join("");
  history.replaceState(null, "", `/r/${id}${location.search}`);
  return id;
}

export function App() {
  const roomId = useMemo(resolveRoomId, []);
  const controller = useMemo(() => new CallController(roomId), [roomId]);
  const state = useSyncExternalStore(controller.subscribe, controller.getState);
  const roomUrl = `${location.origin}/r/${roomId}`;

  switch (state.phase) {
    case "idle":
      return <Setup controller={controller} state={state} roomUrl={roomUrl} />;
    case "full":
      return (
        <Centered>
          <h1>Room is full</h1>
          <p className="muted">This room already has two people in it.</p>
          <a className="button primary" href="/">
            Start a new room
          </a>
        </Centered>
      );
    case "ended":
      return (
        <Centered>
          <h1>Call ended</h1>
          <button className="primary" onClick={() => controller.start()}>
            Rejoin
          </button>
          <a className="button" href="/">
            New room
          </a>
        </Centered>
      );
    default:
      return <CallView controller={controller} state={state} roomUrl={roomUrl} />;
  }
}
