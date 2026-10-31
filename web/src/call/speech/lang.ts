import type { Lang } from "../../../../shared/protocol";

export const LANG_NAMES: Record<Lang, string> = { zh: "中文", en: "English" };
// What each language is recognized as by the browser's SpeechRecognition
export const RECOGNITION_LANG: Record<Lang, string> = { zh: "zh-CN", en: "en-US" };

export function loadLang(): Lang {
  try {
    const saved = localStorage.getItem("lang");
    if (saved === "zh" || saved === "en") return saved;
  } catch {
    // storage blocked: fall back to the browser's language
  }
  return navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en";
}

export function saveLang(lang: Lang): void {
  try {
    localStorage.setItem("lang", lang);
  } catch {
    // storage blocked: the choice just isn't remembered
  }
}

export function loadTranscribeOn(): boolean {
  try {
    return localStorage.getItem("transcribe") === "on";
  } catch {
    return false;
  }
}

export function saveTranscribeOn(on: boolean): void {
  try {
    localStorage.setItem("transcribe", on ? "on" : "off");
  } catch {
    // storage blocked: the choice just isn't remembered
  }
}
