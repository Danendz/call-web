import { Activity, Languages } from "lucide-react";
import { LANGS, type Lang } from "../../../shared/protocol";
import { LANG_NAMES } from "../call/speech/lang";
import { useT } from "../ui/i18n";
import { Sheet } from "../ui/Sheet";
import { SHEET_QUERY, useMediaQuery } from "../ui/useMediaQuery";
import "./settings.css";
import { setShowStats, useShowStats } from "./stats";

/** Settings over the current screen: a bottom sheet on narrow screens, a side panel on the right otherwise. */
export function SettingsSheet({
  open,
  onClose,
  lang,
  onLang,
}: {
  open: boolean;
  onClose: () => void;
  lang: Lang;
  onLang: (lang: Lang) => void;
}) {
  const t = useT();
  const narrow = useMediaQuery(SHEET_QUERY);
  const showStats = useShowStats();

  return (
    <Sheet label={t.settings} open={open} onClose={onClose} side={narrow ? "bottom" : "right"} showTitle>
      <div className="settings">
        <label className="settings-row">
          <span className="row-icon" aria-hidden>
            <Activity size={18} />
          </span>
          <span className="settings-label">{t.stats}</span>
          <input type="checkbox" role="switch" checked={showStats} onChange={(e) => setShowStats(e.target.checked)} />
        </label>
        <label className="settings-row">
          <span className="row-icon" aria-hidden>
            <Languages size={18} />
          </span>
          <span className="settings-label">{t.iSpeak}</span>
          <select value={lang} onChange={(e) => onLang(e.target.value as Lang)}>
            {LANGS.map((l) => (
              <option key={l} value={l}>
                {LANG_NAMES[l]}
              </option>
            ))}
          </select>
        </label>
      </div>
    </Sheet>
  );
}
