import { House, Lock, MonitorSmartphone, PhoneOff, Plus, RotateCcw, Timer, Users } from "lucide-react";
import { useCallback, useEffect, useMemo, useState, useSyncExternalStore, type ReactNode } from "react";
import type { Lang } from "../../shared/protocol";
import { SignIn } from "./account/SignIn";
import { RecoveryCode } from "./account/RecoveryCode";
import { loadSession, recoveryCodeSaved, useSession } from "./account/session";
import { CallController, type CallState } from "./call/controller";
import { CallView } from "./call/CallView";
import { loadLang, saveLang } from "./call/speech/lang";
import { CallOverlay } from "./friends/CallOverlay";
import { FriendsSocket } from "./friends/friendsSocket";
import { Home } from "./friends/Home";
import { Setup } from "./setup/Setup";
import { Centered } from "./ui/Centered";
import { LangContext, STRINGS } from "./ui/i18n";
import { navigate, newRoomPath, useRoute } from "./ui/router";

export function App() {
  const route = useRoute();
  const session = useSession();
  const [lang, setLangState] = useState<Lang>(loadLang);
  const friends = useMemo(() => new FriendsSocket((roomId) => navigate(`/r/${roomId}`, { autoJoin: true })), []);
  const signedIn = session.status === "signed-in";

  const setLang = useCallback((next: Lang) => {
    saveLang(next);
    setLangState(next);
  }, []);

  useEffect(() => {
    void loadSession();
  }, []);

  useEffect(() => {
    if (signedIn) friends.start();
    else friends.stop();
  }, [friends, signedIn]);

  useEffect(() => {
    document.documentElement.lang = lang === "zh" ? "zh-CN" : "en";
  }, [lang]);

  // Signed out (or the server has no accounts), / makes a new room link, as it always has
  useEffect(() => {
    // (keeping ?relay and other test switches)
    if (route.page === "home" && session.status === "signed-out") navigate(newRoomPath() + location.search, { replace: true });
    if (route.page === "signin" && signedIn) navigate("/", { replace: true });
  }, [route.page, session.status, signedIn]);

  let page: ReactNode = null;
  if (session.status === "signed-in" && session.recoveryCode) {
    page = <RecoveryCode code={session.recoveryCode} onDone={recoveryCodeSaved} />;
  } else if (route.page === "room") {
    const onSignIn = session.status === "signed-out" && session.accountsOn ? () => navigate("/signin") : undefined;
    page = <RoomPage key={route.roomId} roomId={route.roomId} autoJoin={route.autoJoin} signedIn={signedIn} onSignIn={onSignIn} onLang={setLang} />;
  } else if (route.page === "signin" && session.status === "signed-out") {
    page = <SignIn />;
  } else if (route.page === "home" && session.status === "signed-in") {
    page = <Home friends={friends} username={session.username} lang={lang} onLang={setLang} />;
  }

  return (
    <LangContext.Provider value={lang}>
      {page}
      {signedIn && <CallOverlay friends={friends} />}
    </LangContext.Provider>
  );
}

/** One room's call, from the pre-join screen to its end. Leaving the page leaves the room. */
function RoomPage(props: { roomId: string; autoJoin: boolean; signedIn: boolean; onSignIn?: () => void; onLang: (lang: Lang) => void }) {
  const { roomId, autoJoin } = props;
  const [controller, setController] = useState<CallController | null>(null);

  useEffect(() => {
    const created = new CallController(roomId);
    setController(created);
    // Answered or rejoined from the friends page: the click there was the go-ahead
    if (autoJoin) void created.start();
    return () => created.dispose();
  }, [roomId]); // autoJoin only matters when the room opens

  return controller && <Room controller={controller} {...props} />;
}

function Room({
  controller,
  roomId,
  signedIn,
  onSignIn,
  onLang,
}: {
  controller: CallController;
  roomId: string;
  signedIn: boolean;
  onSignIn?: () => void;
  onLang: (lang: Lang) => void;
}) {
  const state = useSyncExternalStore(controller.subscribe, controller.getState);
  // "I speak" picked here is the page's language too
  useEffect(() => onLang(state.lang), [onLang, state.lang]);

  return <LangContext.Provider value={state.lang}>{screen(controller, state, `${location.origin}/r/${roomId}`, signedIn, onSignIn)}</LangContext.Provider>;
}

function screen(controller: CallController, state: CallState, roomUrl: string, signedIn: boolean, onSignIn?: () => void) {
  const t = STRINGS[state.lang];
  const home = (primary: boolean) =>
    signedIn ? (
      <button className={primary ? "primary" : ""} onClick={() => navigate("/")}>
        <House size={18} />
        {t.backToFriends}
      </button>
    ) : (
      <button className={primary ? "primary" : ""} onClick={() => navigate(newRoomPath())}>
        <Plus size={18} />
        {primary ? t.startNewRoom : t.newRoom}
      </button>
    );

  switch (state.phase) {
    case "idle":
      return <Setup controller={controller} state={state} onSignIn={onSignIn} />;
    case "full": {
      const [icon, title, body] =
        state.fullReason === "time-limit" ? [<Timer size={30} />, t.timeUp, t.timeUpBody]
        : state.fullReason === "not-invited" ? [<Lock size={30} />, t.privateRoom, t.privateRoomBody]
        : state.fullReason === "elsewhere" ? [<MonitorSmartphone size={30} />, t.joinedElsewhere, t.joinedElsewhereBody]
        : [<Users size={30} />, t.roomFull, t.roomFullBody];
      return (
        <Centered icon={icon}>
          <h1>{title}</h1>
          <p className="muted">{body}</p>
          <div className="actions">{home(true)}</div>
        </Centered>
      );
    }
    case "ended":
      return (
        <Centered icon={<PhoneOff size={30} />}>
          <h1>{t.callEnded}</h1>
          <div className="actions">
            <button className="primary" onClick={() => controller.start()}>
              <RotateCcw size={18} />
              {t.rejoin}
            </button>
            {home(false)}
          </div>
        </Centered>
      );
    default:
      return <CallView controller={controller} state={state} roomUrl={roomUrl} />;
  }
}
