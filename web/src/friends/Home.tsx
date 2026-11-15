import { Ellipsis, Link2, Phone, Shield, UserMinus, UserRound, X } from "lucide-react";
import { useRef, useState, useSyncExternalStore, type FormEvent } from "react";
import type { FriendState, Lang, MissedCall, Presence } from "../../../shared/protocol";
import { api } from "../account/api";
import { AccountSheet } from "../account/AccountSheet";
import { IconButton } from "../ui/IconButton";
import { MenuPanel } from "../ui/MenuPanel";
import { navigate, newRoomPath } from "../ui/router";
import type { FriendsSocket } from "./friendsSocket";
import { useFriendsT } from "./strings";
import "./friends.css";

/** Missed calls from one friend: how many, and the latest. */
interface Missed {
  count: number;
  at: number;
}

const PRESENCE_ORDER: Record<Presence, number> = { online: 0, "in-call": 1, offline: 2 };

/** Friends with a missed call first (latest on top), then by presence and name. */
function sortFriends(friends: FriendState[], missed: Map<string, Missed>): FriendState[] {
  return [...friends].sort(
    (a, b) =>
      (missed.get(b.username)?.at ?? 0) - (missed.get(a.username)?.at ?? 0) ||
      PRESENCE_ORDER[a.presence] - PRESENCE_ORDER[b.presence] ||
      a.username.localeCompare(b.username),
  );
}

function missedByFriend(calls: MissedCall[]): Map<string, Missed> {
  const by = new Map<string, Missed>();
  for (const c of calls) {
    const m = by.get(c.from);
    by.set(c.from, { count: (m?.count ?? 0) + 1, at: Math.max(m?.at ?? 0, c.at) });
  }
  return by;
}

/** Friends with presence and missed calls, requests and private rooms still open: the signed-in start page. */
export function Home({ friends, username, lang, onLang }: { friends: FriendsSocket; username: string; lang: Lang; onLang: (lang: Lang) => void }) {
  const t = useFriendsT();
  const state = useSyncExternalStore(friends.subscribe, friends.getState);
  const [accountOpen, setAccountOpen] = useState(false);
  const lists = state.lists;
  // Requests and blocks answer over /ws/user with a new list; a failed one just leaves the list as it was
  const act = (action: string, name: string) => void api(`/api/friends/${action}`, { username: name }).catch(() => {});
  const locale = lang === "zh" ? "zh-CN" : "en";
  const today = new Intl.DateTimeFormat(locale, { timeStyle: "short" });
  const earlier = new Intl.DateTimeFormat(locale, { dateStyle: "short", timeStyle: "short" });
  // Just the time for today's calls, so the tag fits on a phone
  const when = (at: number) => (new Date(at).toDateString() === new Date().toDateString() ? today : earlier).format(at);
  const missed = missedByFriend(lists?.missed ?? []);

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
        {lists &&
          sortFriends(lists.friends, missed).map((f) => {
            const m = missed.get(f.username);
            return (
              <FriendRow
                key={f.username}
                friend={f}
                missed={m && t.missed(m.count, when(m.at))}
                calling={state.outgoing !== null}
                onCall={() => friends.ring(f.username)}
                onClearMissed={() => friends.missedSeen(f.username)}
                onAct={act}
              />
            );
          })}
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
  missed,
  calling,
  onCall,
  onClearMissed,
  onAct,
}: {
  friend: FriendState;
  /** "Missed call · 19:23", when there is one */
  missed?: string;
  calling: boolean;
  onCall: () => void;
  onClearMissed: () => void;
  onAct: (action: string, name: string) => void;
}) {
  const t = useFriendsT();
  return (
    <div className="row friend">
      <span className="name">
        <span className={`presence ${friend.presence}`} aria-hidden />
        {friend.username}
        <span className="muted small">{t.presence[friend.presence]}</span>
        {missed && (
          <span className="missed small">
            {missed}
            <button type="button" className="missed-clear" aria-label={t.clearMissed(friend.username)} title={t.clearMissed(friend.username)} onClick={onClearMissed}>
              <X size={14} />
            </button>
          </span>
        )}
      </span>
      <span className="row-buttons">
        <FriendMenu name={friend.username} onAct={onAct} />
        <IconButton
          label={t.call(friend.username)}
          variant="active"
          disabled={calling || friend.presence === "offline"}
          onClick={onCall}
        >
          <Phone size={18} />
        </IconButton>
      </span>
    </div>
  );
}

/** ⋯ on a friend: Remove and Block, each asked again before it's done. */
function FriendMenu({ name, onAct }: { name: string; onAct: (action: string, name: string) => void }) {
  const t = useFriendsT();
  const ref = useRef<HTMLSpanElement>(null);
  const [open, setOpen] = useState(false);
  const [confirm, setConfirm] = useState<"remove" | "block" | null>(null);
  const close = () => setOpen(false);
  const toggle = () => {
    setConfirm(null);
    setOpen(!open);
  };
  const act = (action: "remove" | "block") => {
    onAct(action, name);
    close();
  };

  return (
    <span className="friend-menu" ref={ref}>
      <IconButton label={t.moreFor(name)} variant="small" expanded={open} onClick={toggle}>
        <Ellipsis size={18} />
      </IconButton>
      <MenuPanel label={t.moreFor(name)} open={open} onClose={close} within={ref}>
        {confirm === null ? (
          <>
            <button type="button" role="menuitem" className="menu-item" onClick={() => setConfirm("remove")}>
              <UserMinus size={18} />
              <span>{t.remove}</span>
            </button>
            <button type="button" role="menuitem" className="menu-item bad" onClick={() => setConfirm("block")}>
              <Shield size={18} />
              <span>{t.block}</span>
            </button>
          </>
        ) : (
          <div className="menu-confirm" key={confirm}>
            <p>{confirm === "remove" ? t.confirmRemove(name) : t.confirmBlock(name)}</p>
            <div className="row-buttons">
              <button type="button" onClick={() => setConfirm(null)}>
                {t.cancel}
              </button>
              <button type="button" className={confirm === "block" ? "danger" : "primary"} onClick={() => act(confirm)}>
                {confirm === "remove" ? t.remove : t.block}
              </button>
            </div>
          </div>
        )}
      </MenuPanel>
    </span>
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
