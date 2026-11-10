import { Check, Copy } from "lucide-react";
import { useState } from "react";
import { useT } from "./i18n";

/** A value to copy, a room link by default. `label` names it for screen readers. */
export function CopyLink({ url, label }: { url: string; label?: string }) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // clipboard blocked (http / old browser): the link is still selectable
    }
  };
  return (
    <div className="copy-link">
      <input readOnly value={url} onFocus={(e) => e.target.select()} aria-label={label ?? t.roomLink} />
      <button className={copied ? "copied" : ""} onClick={copy}>
        {copied ? <Check size={16} /> : <Copy size={16} />}
        {copied ? t.copied : t.copy}
      </button>
    </div>
  );
}
