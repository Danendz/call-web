// Chat messages into Chinese and English, ported from anixart-web's room translator. Groq's free
// models first, each with its own quota; when all of them are used up, DeepSeek (paid, but cheap for a
// line of chat). Messages are gathered for a moment and sent together in one request

import { LANGS, type Translation } from "../shared/protocol.js";
import { chatCompletion, createQuota, extractJson, type ChatEndpoint } from "./llm.js";

// Best first. Qwen answers without reasoning; gpt-oss can't turn it off, so it reasons as little as it can
const GROQ_MODELS = ["qwen/qwen3.8-27b", "openai/gpt-oss-120b", "openai/gpt-oss-20b"] as const;
const REASONING: Record<(typeof GROQ_MODELS)[number], string> = {
  "qwen/qwen3.8-27b": "none",
  "openai/gpt-oss-120b": "low",
  "openai/gpt-oss-20b": "low",
};

// The other person is waiting on the line: better the original than a slow translation
const TIMEOUT_MS = 8_000;
// A message waits this long for others to go out with it
const BATCH_MS = 300;
// More than this and the batch goes at once, before the reply grows too long to come back in time
const BATCH_MAX = 20;

const PROMPT = [
  "You translate chat messages between two friends on a video call, one speaking Chinese and one English.",
  "Some messages are transcribed from speech and may lack punctuation or contain recognition mistakes; translate what was meant.",
  "Keep the tone, slang, emoji and names; don't explain or add anything. Translate each message on its own.",
  'The user sends {"messages":["...", ...]}. Reply with JSON only, one item per message, "i" being its index: ' +
    '{"items":[{"i":0,"lang":"<ISO 639-1 code of the message language>","zh":"<in Simplified Chinese>","en":"<in English>"}]}',
].join("\n");

// One item of a reply, or null unless it has every language
function readItem(item: Record<string, unknown>): Translation | null {
  const lang = typeof item.lang === "string" ? item.lang.toLowerCase().split(/[-_]/)[0] : "";
  if (!/^[a-z]{2,3}$/.test(lang)) return null;
  const text = {} as Translation["text"];
  for (const l of LANGS) {
    const value = item[l];
    if (typeof value !== "string" || !value.trim()) return null;
    text[l] = value.trim();
  }
  return { lang, text };
}

// The translation of each of `count` messages in a reply; null for one it left out or got wrong
function readTranslations(content: string, count: number): (Translation | null)[] {
  const parsed = extractJson(content) as { items?: unknown } | null;
  const result: (Translation | null)[] = Array.from({ length: count }, () => null);
  for (const item of Array.isArray(parsed?.items) ? parsed.items : []) {
    const i = Number((item as { i?: unknown })?.i);
    if (Number.isInteger(i) && i >= 0 && i < count && result[i] === null) result[i] = readItem(item as Record<string, unknown>);
  }
  return result;
}

interface TranslatorOptions {
  groq?: ChatEndpoint;
  deepseek?: ChatEndpoint & { model: string };
}

class OutOfQuota extends Error {}

export type Translate = (text: string) => Promise<Translation | null>;

function createTranslator({ groq, deepseek }: TranslatorOptions): Translate {
  const withQuota = createQuota(GROQ_MODELS);

  async function ask(endpoint: ChatEndpoint, body: object, count: number) {
    const content = await chatCompletion(endpoint, { ...body, temperature: 0.2 }, TIMEOUT_MS, "Translate");
    const translations = readTranslations(content, count);
    if (translations.every((t) => t === null)) throw new Error("Translate: unreadable reply");
    return translations;
  }

  const messages = (texts: string[]) => [
    { role: "system", content: PROMPT },
    { role: "user", content: JSON.stringify({ messages: texts }) },
  ];

  // One request for the lot; null for each when nothing could translate them
  async function translateAll(texts: string[]): Promise<(Translation | null)[]> {
    if (groq) {
      try {
        return await withQuota(
          (model) => ask(groq, { model, reasoning_effort: REASONING[model], messages: messages(texts) }, texts.length),
          // Every Groq model used up: no waiting, on to DeepSeek
          () => Promise.reject(new OutOfQuota()),
        );
      } catch (error) {
        if (!(error instanceof OutOfQuota)) console.warn((error as Error).message);
      }
    }
    if (deepseek) {
      try {
        // Thinking is on by default for DeepSeek's current models, and billed as output
        return await ask(deepseek, { model: deepseek.model, thinking: { type: "disabled" }, messages: messages(texts) }, texts.length);
      } catch (error) {
        console.warn((error as Error).message);
      }
    }
    return texts.map(() => null);
  }

  let waiting: { text: string; done: (translation: Translation | null) => void }[] = [];
  let timer: NodeJS.Timeout | undefined;

  function send() {
    clearTimeout(timer);
    timer = undefined;
    const batch = waiting;
    waiting = [];
    void translateAll(batch.map((w) => w.text))
      .catch(() => batch.map(() => null))
      .then((translations) => batch.forEach((w, i) => w.done(translations[i] ?? null)));
  }

  // null when nothing could translate it; the chat shows the original
  return (text) =>
    new Promise((resolve) => {
      waiting.push({ text, done: resolve });
      if (waiting.length >= BATCH_MAX) send();
      else timer ??= setTimeout(send, BATCH_MS);
    });
}

// Neither key set (a local run without keys): messages stay as written
export function translatorFromEnv(env = process.env): Translate | null {
  const groq = env.GROQ_API_KEY ? { url: "https://api.groq.com/openai/v1", key: env.GROQ_API_KEY } : undefined;
  const deepseek =
    env.LLM_BASE_URL && env.LLM_API_KEY ? { url: env.LLM_BASE_URL, key: env.LLM_API_KEY, model: env.LLM_MODEL || "deepseek-flash" } : undefined;
  return groq || deepseek ? createTranslator({ groq, deepseek }) : null;
}
