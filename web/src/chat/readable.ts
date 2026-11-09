import type { ChatMessage, Lang } from "../../../shared/protocol";

/** What to show for a message: the other person's in your language with what they wrote under it, your own as written. */
export function readable(m: ChatMessage, myId: string | null, lang: Lang): { text: string; original: string | null } {
  const t = m.translation;
  if (m.from === myId || !t || (t.lang === lang && !hasOtherScript(m.text, lang))) return { text: m.text, original: null };
  return { text: t.text[lang], original: m.text };
}

// One language per message comes back, so "yep 你好" is labelled one of the two; it still needs translating when part of
// it is in the other script. For a Chinese reader a stray "OK" or brand name doesn't count, a real share of English
// words does. Kept the same as native/app/src/chat.rs.
const HAN = /[㐀-䶿一-鿿豈-﫿]|[\u{20000}-\u{2ffff}]/gu;
const LATIN_WORD = /[A-Za-z]+/g;

function hasOtherScript(text: string, lang: Lang): boolean {
  const han = text.match(HAN)?.length ?? 0;
  if (lang === "en") return han > 0;
  // All-caps words are abbreviations like OK or USB, which a Chinese reader knows
  const words = (text.match(LATIN_WORD) ?? []).filter((w) => w !== w.toUpperCase()).length;
  return words > 0 && words / (words + han) >= 0.3;
}
