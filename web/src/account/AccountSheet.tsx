import { LogOut } from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";
import { MAX_PASSWORD_LENGTH, type InvitesResponse, type NewInviteResponse } from "../../../shared/api";
import { LANGS, type Lang } from "../../../shared/protocol";
import { LANG_NAMES } from "../call/speech/lang";
import { CopyLink } from "../ui/CopyLink";
import { Sheet } from "../ui/Sheet";
import { ApiFailure, api } from "./api";
import { signOut, signedOut } from "./session";
import { useAccountT } from "./strings";
import "./account.css";

/** Language, invite codes, password, blocked users, signing out and deleting the account. */
export function AccountSheet({
  open,
  onClose,
  username,
  blocked,
  lang,
  onLang,
}: {
  open: boolean;
  onClose: () => void;
  username: string;
  blocked: string[];
  lang: Lang;
  onLang: (lang: Lang) => void;
}) {
  const t = useAccountT();
  return (
    <Sheet label={`${t.account}: ${username}`} open={open} onClose={onClose} side="right" showTitle>
      <div className="account-sheet">
        <label className="row">
          <span>{t.language}</span>
          <select value={lang} onChange={(e) => onLang(e.target.value as Lang)}>
            {LANGS.map((l) => (
              <option key={l} value={l}>
                {LANG_NAMES[l]}
              </option>
            ))}
          </select>
        </label>
        {open && <Invites />}
        <ChangePassword />
        {blocked.length > 0 && (
          <section>
            <h2>{t.blocked}</h2>
            {blocked.map((name) => (
              <div className="row" key={name}>
                <span>{name}</span>
                <button onClick={() => void api("/api/friends/unblock", { username: name }).catch(() => {})}>{t.unblock}</button>
              </div>
            ))}
          </section>
        )}
        <button onClick={() => void signOut()}>
          <LogOut size={18} />
          {t.signOut}
        </button>
        <DeleteAccount />
      </div>
    </Sheet>
  );
}

function useErrorText() {
  const t = useAccountT();
  return (err: unknown) => t.errors[err instanceof ApiFailure ? err.code : "unavailable"];
}

function Invites() {
  const t = useAccountT();
  const errorText = useErrorText();
  const [count, setCount] = useState<InvitesResponse | null>(null);
  const [code, setCode] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api<InvitesResponse>("/api/invites").then(setCount, () => {});
  }, [code]);

  const create = async () => {
    setError(null);
    try {
      setCode((await api<NewInviteResponse>("/api/invites", {})).code);
    } catch (err) {
      setError(errorText(err));
    }
  };

  return (
    <section>
      <h2>{t.invites}</h2>
      {count && <p className="muted small">{t.invitesBody(count.unused, count.max)}</p>}
      {code && (
        <>
          <CopyLink url={code} label={t.inviteCode} />
          <p className="muted small">{t.shownOnce}</p>
        </>
      )}
      {error && <p className="error small">{error}</p>}
      <button onClick={create} disabled={!!count && count.unused >= count.max}>
        {t.newInvite}
      </button>
    </section>
  );
}

function ChangePassword() {
  const t = useAccountT();
  const errorText = useErrorText();
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    try {
      await api("/api/auth/password", { password: current, newPassword: next });
      setCurrent("");
      setNext("");
      setMessage({ ok: true, text: t.passwordChanged });
    } catch (err) {
      setMessage({ ok: false, text: errorText(err) });
    }
  };

  return (
    <section>
      <h2>{t.changePassword}</h2>
      <form onSubmit={submit}>
        <label>
          <span>{t.currentPassword}</span>
          <input type="password" autoComplete="current-password" value={current} onChange={(e) => setCurrent(e.target.value)} maxLength={MAX_PASSWORD_LENGTH} required />
        </label>
        <label>
          <span>{t.newPassword}</span>
          <input type="password" autoComplete="new-password" value={next} onChange={(e) => setNext(e.target.value)} minLength={10} maxLength={MAX_PASSWORD_LENGTH} required />
          <small className="muted">{t.passwordRule}</small>
        </label>
        {message && <p className={`small ${message.ok ? "muted" : "error"}`}>{message.text}</p>}
        <button type="submit">{t.changePassword}</button>
      </form>
    </section>
  );
}

function DeleteAccount() {
  const t = useAccountT();
  const errorText = useErrorText();
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    try {
      await api("/api/auth/delete", { password });
      signedOut();
    } catch (err) {
      setError(errorText(err));
    }
  };

  return (
    <section>
      <h2>{t.deleteAccount}</h2>
      <form onSubmit={submit}>
        <p className="muted small">{t.deleteBody}</p>
        <input type="password" autoComplete="current-password" aria-label={t.password} value={password} onChange={(e) => setPassword(e.target.value)} maxLength={MAX_PASSWORD_LENGTH} required />
        {error && <p className="error small">{error}</p>}
        <button className="danger" type="submit">
          {t.deleteAccount}
        </button>
      </form>
    </section>
  );
}
