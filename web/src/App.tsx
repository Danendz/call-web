import { PhoneOff, RotateCcw, Plus, Users } from "lucide-react";
import { useEffect, useMemo, useSyncExternalStore } from "react";
import { ROOM_ID_RE } from "../../shared/protocol";
import { CallController, type CallState } from "./call/controller";
import { CallView } from "./call/CallView";
import { Setup } from "./setup/Setup";
import { Centered } from "./ui/Centered";
import { LangContext, STRINGS } from "./ui/i18n";

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

  useEffect(() => {
    document.documentElement.lang = state.lang === "zh" ? "zh-CN" : "en";
  }, [state.lang]);

  return <LangContext.Provider value={state.lang}>{screen(controller, state, roomUrl)}</LangContext.Provider>;
}

function screen(controller: CallController, state: CallState, roomUrl: string) {
  const t = STRINGS[state.lang];
  switch (state.phase) {
    case "idle":
      return <Setup controller={controller} state={state} roomUrl={roomUrl} />;
    case "full":
      return (
        <Centered icon={<Users size={30} />}>
          <h1>{t.roomFull}</h1>
          <p className="muted">{t.roomFullBody}</p>
          <div className="actions">
            <a className="button primary" href="/">
              <Plus size={18} />
              {t.startNewRoom}
            </a>
          </div>
        </Centered>
      );
    case "ended":
      return (
        <Centered icon={<PhoneOff size={30} />}>
          <h1>{t.callEnded}</h1>
          <div className="actions">
            <button className="primary" onClick={() => controller.start()}>
              <RotateCcw size={18} />
              {t.rejoin}
            </button>
            <a className="button" href="/">
              <Plus size={18} />
              {t.newRoom}
            </a>
          </div>
        </Centered>
      );
    default:
      return <CallView controller={controller} state={state} roomUrl={roomUrl} />;
  }
}
