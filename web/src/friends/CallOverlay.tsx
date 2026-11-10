import { Phone, PhoneOff, X } from "lucide-react";
import { useEffect, useSyncExternalStore } from "react";
import type { FriendsSocket } from "./friendsSocket";
import { startRinging, stopRinging } from "./ringtone";
import { useFriendsT } from "./strings";
import "./friends.css";

const NOTICE_SHOWN_MS = 6_000;

/** A call coming in, your ring going out, or how it ended: shown over whatever page is open. */
export function CallOverlay({ friends }: { friends: FriendsSocket }) {
  const t = useFriendsT();
  const { incoming, outgoing, notice } = useSyncExternalStore(friends.subscribe, friends.getState);
  const call = incoming[0];

  useEffect(() => {
    if (!call) return;
    startRinging();
    return stopRinging;
  }, [call]);

  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => friends.dismissNotice(), NOTICE_SHOWN_MS);
    return () => clearTimeout(timer);
  }, [friends, notice]);

  if (call) {
    return (
      <div className="call-overlay" role="alertdialog" aria-label={t.incoming(call.from)}>
        <span className="ring-pulse" aria-hidden>
          <Phone size={22} />
        </span>
        <strong>{t.incoming(call.from)}</strong>
        <span className="row-buttons">
          <button className="danger" onClick={() => friends.answer(call.callId, false)}>
            <PhoneOff size={18} />
            {t.decline}
          </button>
          <button className="primary" onClick={() => friends.answer(call.callId, true)}>
            <Phone size={18} />
            {t.accept}
          </button>
        </span>
      </div>
    );
  }
  if (outgoing) {
    return (
      <div className="call-overlay" role="status">
        <span className="ring-pulse" aria-hidden>
          <Phone size={22} />
        </span>
        <strong>{t.calling(outgoing.to)}</strong>
        <button className="danger" onClick={() => friends.cancel()}>
          <PhoneOff size={18} />
          {t.cancel}
        </button>
      </div>
    );
  }
  const text = notice && t.ended[notice.reason](notice.with);
  if (text) {
    return (
      <div className="call-overlay notice" role="status">
        <span>{text}</span>
        <button className="icon-btn small" aria-label={t.dismiss} title={t.dismiss} onClick={() => friends.dismissNotice()}>
          <X size={18} />
        </button>
      </div>
    );
  }
  return null;
}
