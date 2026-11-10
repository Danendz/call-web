import { KeyRound } from "lucide-react";
import { useState, type FormEvent } from "react";
import { MAX_PASSWORD_LENGTH } from "../../../shared/api";
import { navigate, newRoomPath } from "../ui/router";
import { ApiFailure } from "./api";
import { startSession } from "./session";
import { useAccountT } from "./strings";
import "./account.css";

type Mode = "signin" | "register" | "recover";

/** Sign in, create an account with an invite code, or reset the password with the recovery code. */
export function SignIn() {
  const t = useAccountT();
  const [mode, setMode] = useState<Mode>("signin");
  const [fields, setFields] = useState({ username: "", password: "", invite: "", recoveryCode: "" });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const field = (name: keyof typeof fields) => ({
    value: fields[name],
    onChange: (e: { target: { value: string } }) => setFields({ ...fields, [name]: e.target.value }),
  });

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    const { username, password, invite, recoveryCode } = fields;
    try {
      // Signed in, the page moves on by itself (App)
      if (mode === "signin") await startSession("/api/auth/login", { username, password });
      else if (mode === "register") await startSession("/api/auth/register", { username, password, invite });
      else await startSession("/api/auth/recover", { username, password, recoveryCode });
    } catch (err) {
      setError(t.errors[err instanceof ApiFailure ? err.code : "unavailable"]);
    } finally {
      setBusy(false);
    }
  };

  const switchTo = (next: Mode) => {
    setMode(next);
    setError(null);
  };

  return (
    <div className="account-page">
      <form className="account-card" onSubmit={submit}>
        <div className="account-icon" aria-hidden>
          <KeyRound size={26} />
        </div>
        <div className="tabs" role="tablist">
          {(["signin", "register"] as const).map((m) => (
            <button key={m} type="button" role="tab" aria-selected={mode === m} className={mode === m ? "active" : ""} onClick={() => switchTo(m)}>
              {m === "signin" ? t.signIn : t.createAccount}
            </button>
          ))}
        </div>

        {mode === "register" && <p className="muted small">{t.inviteOnly}</p>}
        {mode === "recover" && <p className="muted small">{t.recoverHelp}</p>}

        <label>
          <span>{t.username}</span>
          <input {...field("username")} autoComplete="username" autoCapitalize="none" spellCheck={false} maxLength={20} required />
          {mode === "register" && <small className="muted">{t.usernameRule}</small>}
        </label>
        {mode === "register" && (
          <label>
            <span>{t.inviteCode}</span>
            <input {...field("invite")} autoComplete="off" autoCapitalize="characters" spellCheck={false} maxLength={64} required />
          </label>
        )}
        {mode === "recover" && (
          <label>
            <span>{t.recoveryCode}</span>
            <input {...field("recoveryCode")} autoComplete="off" autoCapitalize="characters" spellCheck={false} maxLength={64} required />
          </label>
        )}
        <label>
          <span>{mode === "recover" ? t.newPassword : t.password}</span>
          <input
            {...field("password")}
            type="password"
            autoComplete={mode === "signin" ? "current-password" : "new-password"}
            minLength={mode === "signin" ? undefined : 10}
            maxLength={MAX_PASSWORD_LENGTH}
            required
          />
          {mode !== "signin" && <small className="muted">{t.passwordRule}</small>}
        </label>

        {error && (
          <p className="error small" role="alert">
            {error}
          </p>
        )}
        <button className="primary" type="submit" disabled={busy}>
          {mode === "signin" ? t.signIn : mode === "register" ? t.createAccount : t.changePassword}
        </button>

        {mode === "signin" && (
          <button type="button" className="link" onClick={() => switchTo("recover")}>
            {t.forgotPassword}
          </button>
        )}
        <button type="button" className="link" onClick={() => navigate(newRoomPath())}>
          {t.withoutAccount}
        </button>
      </form>
    </div>
  );
}
