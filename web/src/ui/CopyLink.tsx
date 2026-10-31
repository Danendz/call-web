import { useState } from "react";

export function CopyLink({ url }: { url: string }) {
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
      <input readOnly value={url} onFocus={(e) => e.target.select()} />
      <button onClick={copy}>{copied ? "Copied" : "Copy"}</button>
    </div>
  );
}
