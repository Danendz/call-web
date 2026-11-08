import { Download, Laptop, Monitor, Smartphone } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useT } from "../ui/i18n";

// The installers' fixed names on our own server (server/updates.ts); each release replaces them
const APPS = [
  { os: "Windows", file: "CallDanya-win-Setup.exe", icon: <Monitor size={20} />, note: "noteWindows" },
  { os: "macOS", file: "CallDanya-osx-Setup.pkg", icon: <Laptop size={20} />, note: "noteMac" },
  { os: "Android", file: "call-danya.apk", icon: <Smartphone size={20} />, note: "noteAndroid" },
] as const;

/** The Download button and its popover with the native apps. Closes on outside tap and Esc. */
export function DownloadApp() {
  const t = useT();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("pointerdown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div className="download-app" ref={ref}>
      <button type="button" className="download-toggle" aria-expanded={open} aria-haspopup="menu" onClick={() => setOpen(!open)}>
        <Download size={16} />
        {t.download}
      </button>
      <div className={`download-menu ${open ? "open" : ""}`} role="menu" aria-hidden={!open}>
        {APPS.map((app) => (
          <a
            key={app.os}
            className="download-item"
            role="menuitem"
            href={`/updates/${app.file}`}
            download
            tabIndex={open ? 0 : -1}
            onClick={() => setOpen(false)}
          >
            {app.icon}
            <span>
              <strong>{app.os}</strong>
              <small>{t[app.note]}</small>
            </span>
          </a>
        ))}
      </div>
    </div>
  );
}
