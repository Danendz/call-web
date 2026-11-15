import { Activity, AudioLines, Captions, ChevronLeft, ChevronRight, Film, Languages, Mic, Sparkles, Subtitles, UserRound, Video } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import type { VideoCodec } from "../call/codec";
import { LANGS, type Lang } from "../../../shared/protocol";
import { AccountPage } from "../account/AccountPage";
import { useAccountT } from "../account/strings";
import { LANG_NAMES } from "../call/speech/lang";
import { ModelAction } from "../translate/ModelAction";
import { IconButton } from "../ui/IconButton";
import { useT } from "../ui/i18n";
import { Sheet } from "../ui/Sheet";
import { SHEET_QUERY, useMediaQuery } from "../ui/useMediaQuery";
import "./settings.css";
import { setVideoCodec, useVideoCodec } from "./codec";
import { setNoiseSuppression, useNoiseSuppression } from "./noise";
import { setSharpenVideo, useSharpenVideo } from "./sharpen";
import { setShowStats, useShowStats } from "./stats";
import { setSubtitles, useSubtitles } from "./subtitles";

export type SettingsPage = "root" | "audio" | "video" | "speech" | "account";

/** The signed-in account, for the Account page. */
export interface SettingsAccount {
  username: string;
  blocked: string[];
}

/**
 * Settings over the current screen: a bottom sheet on narrow screens, a side panel on the right otherwise. The first
 * page lists the categories; each opens its own page, and Back or Esc goes up again.
 */
export function SettingsSheet({
  open,
  onClose,
  lang,
  onLang,
  account,
  startPage = "root",
}: {
  open: boolean;
  onClose: () => void;
  lang: Lang;
  onLang: (lang: Lang) => void;
  account?: SettingsAccount;
  /** The page it opens on, each time it opens */
  startPage?: SettingsPage;
}) {
  const t = useT();
  const at = useAccountT();
  const narrow = useMediaQuery(SHEET_QUERY);
  const [page, setPage] = useState<SettingsPage>(startPage);
  // Going in slides the page from the right, going back from the left
  const [back, setBack] = useState(false);

  useEffect(() => {
    if (open) {
      setPage(startPage);
      setBack(false);
    }
  }, [open, startPage]);

  const go = (next: SettingsPage) => {
    setBack(next === "root");
    setPage(next);
  };
  const titles: Record<SettingsPage, string> = { root: t.settings, audio: t.audio, video: t.video, speech: t.speech, account: at.account };

  return (
    <Sheet
      label={titles[page]}
      open={open}
      onClose={onClose}
      side={narrow ? "bottom" : "right"}
      onEscape={(e) => {
        if (page === "root") return;
        e.preventDefault();
        go("root");
      }}
    >
      <header className="settings-head">
        {page !== "root" && (
          <IconButton label={t.back} variant="small" onClick={() => go("root")}>
            <ChevronLeft size={20} />
          </IconButton>
        )}
        <span aria-hidden>{titles[page]}</span>
      </header>
      <div className={`settings ${page === "root" && !back ? "" : back ? "from-left" : "from-right"}`} key={page}>
        {page === "root" && <RootPage lang={lang} account={account} onOpen={go} />}
        {page === "audio" && <AudioPage />}
        {page === "video" && <VideoPage />}
        {page === "speech" && <SpeechPage lang={lang} onLang={onLang} />}
        {page === "account" && account && <AccountPage username={account.username} blocked={account.blocked} />}
      </div>
    </Sheet>
  );
}

function RootPage({ lang, account, onOpen }: { lang: Lang; account?: SettingsAccount; onOpen: (page: SettingsPage) => void }) {
  const t = useT();
  const at = useAccountT();
  const noiseSuppression = useNoiseSuppression();
  const videoCodec = useVideoCodec();
  const subtitles = useSubtitles();
  const onOff = (on: boolean) => (on ? t.on : t.off);
  const codec = videoCodec === "auto" ? t.codecAuto : videoCodec === "h264" ? "H.264" : "VP8";
  return (
    <>
      <PageRow icon={<Mic size={18} />} label={t.audio} summary={`${t.noiseSuppression}: ${onOff(noiseSuppression)}`} onOpen={() => onOpen("audio")} />
      <PageRow icon={<Video size={18} />} label={t.video} summary={`${t.videoCodec}: ${codec}`} onOpen={() => onOpen("video")} />
      <PageRow
        icon={<Languages size={18} />}
        label={t.speech}
        summary={`${LANG_NAMES[lang]} · ${t.liveSubtitles}: ${onOff(subtitles)}`}
        onOpen={() => onOpen("speech")}
      />
      {account && <PageRow icon={<UserRound size={18} />} label={at.account} summary={account.username} onOpen={() => onOpen("account")} />}
    </>
  );
}

/** A category on the first page, with its settings at a glance */
function PageRow({ icon, label, summary, onOpen }: { icon: ReactNode; label: string; summary: string; onOpen: () => void }) {
  return (
    <button type="button" className="settings-row page" onClick={onOpen}>
      <span className="row-icon" aria-hidden>
        {icon}
      </span>
      <span className="settings-label">
        {label}
        <span className="settings-detail">{summary}</span>
      </span>
      <ChevronRight size={18} className="row-arrow" aria-hidden />
    </button>
  );
}

function AudioPage() {
  const t = useT();
  const noiseSuppression = useNoiseSuppression();
  return (
    <label className="settings-row">
      <span className="row-icon" aria-hidden>
        <AudioLines size={18} />
      </span>
      <span className="settings-label">
        {t.noiseSuppression}
        <span className="settings-detail">{t.noiseSuppressionDetail}</span>
      </span>
      <input type="checkbox" role="switch" checked={noiseSuppression} onChange={(e) => setNoiseSuppression(e.target.checked)} />
    </label>
  );
}

function VideoPage() {
  const t = useT();
  const videoCodec = useVideoCodec();
  const sharpenVideo = useSharpenVideo();
  const showStats = useShowStats();
  return (
    <>
      <label className="settings-row">
        <span className="row-icon" aria-hidden>
          <Sparkles size={18} />
        </span>
        <span className="settings-label">
          {t.sharpenVideo}
          <span className="settings-detail">{t.sharpenVideoDetail}</span>
        </span>
        <input type="checkbox" role="switch" checked={sharpenVideo} onChange={(e) => setSharpenVideo(e.target.checked)} />
      </label>
      <label className="settings-row">
        <span className="row-icon" aria-hidden>
          <Film size={18} />
        </span>
        <span className="settings-label">
          {t.videoCodec}
          <span className="settings-detail">{t.videoCodecDetail}</span>
        </span>
        <select value={videoCodec} onChange={(e) => setVideoCodec(e.target.value as VideoCodec)}>
          <option value="auto">{t.codecAuto}</option>
          <option value="h264">H.264</option>
          <option value="vp8">VP8</option>
        </select>
      </label>
      <label className="settings-row">
        <span className="row-icon" aria-hidden>
          <Activity size={18} />
        </span>
        <span className="settings-label">{t.stats}</span>
        <input type="checkbox" role="switch" checked={showStats} onChange={(e) => setShowStats(e.target.checked)} />
      </label>
    </>
  );
}

function SpeechPage({ lang, onLang }: { lang: Lang; onLang: (lang: Lang) => void }) {
  const t = useT();
  const subtitles = useSubtitles();
  return (
    <>
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
      <label className="settings-row">
        <span className="row-icon" aria-hidden>
          <Captions size={18} />
        </span>
        <span className="settings-label">{t.liveSubtitles}</span>
        <input type="checkbox" role="switch" checked={subtitles} onChange={(e) => setSubtitles(e.target.checked)} />
      </label>
      {/* The model for the language picked above, so it follows "I speak" */}
      <div className="settings-row">
        <span className="row-icon" aria-hidden>
          <Subtitles size={18} />
        </span>
        <span className="settings-label">{t.liveTranslation}</span>
        <ModelAction lang={lang} />
      </div>
    </>
  );
}
