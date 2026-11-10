import { Activity, AudioLines, Captions, Languages, Subtitles } from "lucide-react";
import { LANGS, type Lang } from "../../../shared/protocol";
import { LANG_NAMES } from "../call/speech/lang";
import { ModelAction } from "../translate/ModelAction";
import { useT } from "../ui/i18n";
import { Sheet } from "../ui/Sheet";
import { SHEET_QUERY, useMediaQuery } from "../ui/useMediaQuery";
import "./settings.css";
import { setNoiseSuppression, useNoiseSuppression } from "./noise";
import { setShowStats, useShowStats } from "./stats";
import { setSubtitles, useSubtitles } from "./subtitles";

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
  const noiseSuppression = useNoiseSuppression();
  const subtitles = useSubtitles();

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
            <Captions size={18} />
          </span>
          <span className="settings-label">{t.liveSubtitles}</span>
          <input type="checkbox" role="switch" checked={subtitles} onChange={(e) => setSubtitles(e.target.checked)} />
        </label>
        {/* The model for the language picked below, so it follows "I speak" */}
        <div className="settings-row">
          <span className="row-icon" aria-hidden>
            <Subtitles size={18} />
          </span>
          <span className="settings-label">{t.liveTranslation}</span>
          <ModelAction lang={lang} />
        </div>
        <label className="settings-row">
          <span className="row-icon" aria-hidden>
            <AudioLines size={18} />
          </span>
          <span className="settings-label">
            {t.noiseSuppression}
            <span className="settings-detail">{t.noiseSuppressionDetail}</span>
          </span>
          <input
            type="checkbox"
            role="switch"
            checked={noiseSuppression}
            onChange={(e) => setNoiseSuppression(e.target.checked)}
          />
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
