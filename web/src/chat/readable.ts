import type { ChatMessage, Lang } from "../../../shared/protocol";

/** What to show for a message: the other person's in your language with what they wrote under it, your own as written. */
export function readable(m: ChatMessage, myId: string | null, lang: Lang): { text: string; original: string | null } {
  const t = m.translation;
  if (m.from === myId || !t || t.lang === lang) return { text: m.text, original: null };
  return { text: t.text[lang], original: m.text };
}
