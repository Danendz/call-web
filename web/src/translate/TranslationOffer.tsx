import { useEffect, useRef, useState } from "react";
import type { Lang } from "../../../shared/protocol";
import { LANG_NAMES } from "../call/speech/lang";
import { useSubtitles } from "../settings/subtitles";
import { useT } from "../ui/i18n";
import { ModelAction } from "./ModelAction";
import { pairFor, sizeMb, useModelStatus } from "./models";
import "./translate.css";

const KEY = "translation-offer-dismissed";

function loadDismissed(): Lang[] {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "[]") as Lang[];
  } catch {
    return [];
  }
}

function saveDismissed(langs: Lang[]): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(langs));
  } catch {
    // storage blocked: asked again next time
  }
}

/**
 * Offers the translation model your language needs while live subtitles are on and it isn't downloaded. "Not now"
 * holds for that language until you pick a language again.
 */
export function TranslationOffer({ lang }: { lang: Lang }) {
  const t = useT();
  const subtitles = useSubtitles();
  const pair = pairFor(lang);
  const status = useModelStatus(pair);
  const [dismissed, setDismissed] = useState(loadDismissed);
  const before = useRef(lang);
  useEffect(() => {
    if (before.current === lang) return;
    before.current = lang;
    setDismissed((d) => {
      const next = d.filter((l) => l !== lang);
      saveDismissed(next);
      return next;
    });
  }, [lang]);

  const open = subtitles && status.kind !== "checking" && status.kind !== "ready" && !dismissed.includes(lang);
  if (!open) return null;
  return (
    <div className="translation-offer" role="dialog" aria-label={t.liveTranslation}>
      <p>{status.kind === "failed" ? t.downloadFailed : t.liveTranslationOffer(LANG_NAMES[lang], sizeMb(pair))}</p>
      <div className="translation-offer-actions">
        {status.kind !== "downloading" && (
          <button
            type="button"
            onClick={() => {
              const next = [...dismissed, lang];
              saveDismissed(next);
              setDismissed(next);
            }}
          >
            {t.notNow}
          </button>
        )}
        <ModelAction lang={lang} primary />
      </div>
    </div>
  );
}
