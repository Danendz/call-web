import { Ellipsis, Phone, Shield, UserMinus, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { FriendState } from "../../../shared/protocol";
import { IconButton } from "../ui/IconButton";
import { MenuPanel } from "../ui/MenuPanel";
import { useFriendsT } from "./strings";

/** A friend on the friends list: presence, missed calls, ⋯ and call. */
export function FriendRow({
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
  // Counts their comings online: a new key puts the dot back in, and its ping plays again
  const [pings, setPings] = useState(0);
  const wasOnline = useRef(friend.presence === "online");
  useEffect(() => {
    const online = friend.presence === "online";
    if (online && !wasOnline.current) setPings((n) => n + 1);
    wasOnline.current = online;
  }, [friend.presence]);
  return (
    <div className="row friend" data-flip={friend.username}>
      <span className="name">
        <span key={pings} className={`presence ${friend.presence}${pings ? " ping" : ""}`} aria-hidden />
        {friend.username}
        <span className="muted small">{t.presence[friend.presence]}</span>
        {missed && (
          <span className="missed small">
            {missed}
            <button
              type="button"
              className="missed-clear"
              aria-label={t.clearMissed(friend.username)}
              title={t.clearMissed(friend.username)}
              onClick={onClearMissed}
            >
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
