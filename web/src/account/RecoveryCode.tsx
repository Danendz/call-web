import { ShieldCheck } from "lucide-react";
import { CopyLink } from "../ui/CopyLink";
import { useAccountT } from "./strings";

/** Shows a new recovery code once, until the user says they saved it. */
export function RecoveryCode({ code, onDone }: { code: string; onDone: () => void }) {
  const t = useAccountT();
  return (
    <div className="account-page">
      <div className="account-card">
        <div className="account-icon" aria-hidden>
          <ShieldCheck size={26} />
        </div>
        <h1>{t.saveRecoveryTitle}</h1>
        <p className="muted">{t.saveRecoveryBody}</p>
        <CopyLink url={code} label={t.recoveryCode} />
        <button className="primary" onClick={onDone}>
          {t.savedIt}
        </button>
      </div>
    </div>
  );
}
