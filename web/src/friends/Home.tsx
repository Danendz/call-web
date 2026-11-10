import { Ellipsis, Link2, Phone, UserRound } from "lucide-react";
import { useState, useSyncExternalStore, type FormEvent } from "react";
import type { FriendState, Lang } from "../../../shared/protocol";
import { api } from "../account/api";
import { AccountSheet } from "../account/AccountSheet";
import { IconButton } from "../ui/IconButton";
import { navigate, newRoomPath } from "../ui/router";
import type { FriendsSocket } from "./friendsSocket";
import { useFriendsT } from "./strings";
import "./friends.css";

/** Friends with presence, requests, missed calls and private rooms still open: the signed-in start page. */
export function Home({ friends, username, lang, onLang }: { friends: FriendsSocket; username: string; lang: Lang; onLang: (lang: Lang) => void }) {
  const t = useFriendsT();
  const state = useSyncExternalStore(friends.subscribe, friends.getState);
  const [accountOpen, setAccountOpen] = useState(false);
  const lists = state.lists;
  // Requests and blocks answer over /ws/user with a new list; a failed one just leaves the list as it was
  const act = (action: string, name: string) => void api(`/api/friends/${action}`, { username: name }).catch(() => {});
  const time = new Intl.DateTimeFormat(lang === "zh" ? "zh-CN" : "en", { dateStyle: "short", timeStyle: "short" });

  return (
    <div className="home">
      <header className="home-head">
        <h1>{t.friends}</h1>
        <div className="home-actions">
          <button onClick={() => navigate(newRoomPath())}>
            <Link2 size={18} />
            {t.newRoomLink}
          </button>
          <IconButton label={username} onClick={() => setAccountOpen(true)}>
            <UserRound size={20} />
          </IconButton>
        </div>
      </header>

      {!state.connected && <p className="muted small">{t.reconnecting}</p>}

      {lists && lists.missed.length > 0 && (
        <section className="home-card">
          <div className="card-head">
            <h2>{t.missedCalls}</h2>
            <button className="link" onClick={() => friends.missedSeen()}>
              {t.clear}
            </button>
          </div>
          {lists.missed.map((m) => (
            <div className="row" key={`${m.from}-${m.at}`}>
              <span className="name">{m.from}</span>
              <span className="muted small">{time.format(m.at)}</span>
            </div>
          ))}
        </section>
      )}

      {lists && lists.rooms.length > 0 && (
        <section className="home-card">
          <h2>{t.rooms}</h2>
          {lists.rooms.map((r) => (
            <div className="row" key={r.roomId}>
              <span>{t.stillInCall(r.with)}</span>
              <button className="primary" onClick={() => navigate(`/r/${r.roomId}`, { autoJoin: true })}>
                {t.rejoin}
              </button>
            </div>
          ))}
        </section>
      )}

      {lists && lists.incoming.length > 0 && (
        <section className="home-card">
          <h2>{t.requests}</h2>
          {lists.incoming.map((name) => (
            <div className="row" key={name}>
              <span className="name">{name}</span>
              <span className="row-buttons">
                <button className="primary" onClick={() => act("accept", name)}>
                  {t.accept}
                </button>
                <button onClick={() => act("remove", name)}>{t.decline}</button>
              </span>
            </div>
          ))}
        </section>
      )}

      <section className="home-card">
        {lists && lists.friends.length === 0 && <p className="muted small">{t.noFriends}</p>}
        {lists?.friends.map((f) => (
          <FriendRow key={f.username} friend={f} calling={state.outgoing !== null} onCall={() => friends.ring(f.username)} onAct={act} />
        ))}
        {lists?.outgoing.map((name) => (
          <div className="row" key={name}>
            <span className="name">
              {name}
              <span className="muted small">{t.waiting}</span>
            </span>
            <button onClick={() => act("remove", name)}>{t.withdraw}</button>
          </div>
        ))}
        <AddFriend />
      </section>

      <AccountSheet
        open={accountOpen}
        onClose={() => setAccountOpen(false)}
        username={username}
        blocked={lists?.blocked ?? []}
        lang={lang}
        onLang={onLang}
      />
    </div>
  );
}

function FriendRow({
  friend,
  calling,
  onCall,
  onAct,
}: {
  friend: FriendState;
  calling: boolean;
  onCall: () => void;
  onAct: (action: string, name: string) => void;
}) {
  const t = useFriendsT();
  const [more, setMore] = useState(false);
  return (
    <div className="friend">
      <div className="row">
        <span className="name">
          <span className={`presence ${friend.presence}`} aria-hidden />
          {friend.username}
          <span className="muted small">{t.presence[friend.presence]}</span>
        </span>
        <span className="row-buttons">
          <IconButton label={t.moreFor(friend.username)} variant="small" expanded={more} onClick={() => setMore(!more)}>
            <Ellipsis size={18} />
          </IconButton>
          <IconButton label={t.call(friend.username)} variant="active" disabled={calling} onClick={onCall}>
            <Phone size={18} />
          </IconButton>
        </span>
      </div>
      {more && (
        <div className="row-buttons more">
          <button onClick={() => onAct("remove", friend.username)}>{t.remove}</button>
          <button className="danger" onClick={() => onAct("block", friend.username)}>
            {t.block}
          </button>
        </div>
      )}
    </div>
  );
}

function AddFriend() {
  const t = useFriendsT();
  const [name, setName] = useState("");
  const [sent, setSent] = useState(false);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    try {
      await api("/api/friends/request", { username: name.trim().toLowerCase() });
      setName("");
      setSent(true);
    } catch {
      setSent(false);
    }
  };

  return (
    <form className="add-friend" onSubmit={submit}>
      <input
        value={name}
        onChange={(e) => (setName(e.target.value), setSent(false))}
        placeholder={t.addFriend}
        aria-label={t.addFriend}
        autoCapitalize="none"
        spellCheck={false}
        maxLength={20}
        required
      />
      <button type="submit">{t.send}</button>
      {sent && <p className="muted small">{t.requestSent}</p>}
    </form>
  );
}
