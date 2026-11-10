import type { Lang } from "../../../shared/protocol";
import { useT } from "../ui/i18n";
import { download, pairFor, remove, sizeMb, useModelStatus } from "./models";

/** The button for the translation model your language needs: download it, see how far along, or delete it. */
export function ModelAction({ lang, primary = false }: { lang: Lang; primary?: boolean }) {
  const t = useT();
  const pair = pairFor(lang);
  const status = useModelStatus(pair);
  switch (status.kind) {
    case "checking":
      return null;
    case "downloading":
      return (
        <span className="model-progress" role="progressbar" aria-valuenow={status.percent}>
          {t.downloading(status.percent)}
        </span>
      );
    case "ready":
      return (
        <button type="button" className="model-action" onClick={() => void remove(pair)}>
          {t.remove}
        </button>
      );
    default:
      return (
        <button type="button" className={`model-action ${primary ? "primary" : ""}`} onClick={() => void download(pair)}>
          {status.kind === "failed" ? t.retry : t.downloadSize(sizeMb(pair))}
        </button>
      );
  }
}
