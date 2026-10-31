// Calls to OpenAI-compatible APIs (Groq, DeepSeek) and Groq's per-model quotas. Taken from anixart-web's
// packages/llm, trimmed to what translation and transcription use.

export class HttpError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly retryAfterS?: number,
    readonly body = "",
  ) {
    super(message);
  }
}

export async function failIfNotOk(res: Response, what: string): Promise<void> {
  if (res.ok) return;
  const retryAfter = Number(res.headers.get("retry-after"));
  const body = (await res.text()).slice(0, 1000);
  throw new HttpError(
    `${what} ${res.status}: ${body.slice(0, 300)}`,
    res.status,
    Number.isFinite(retryAfter) && retryAfter > 0 ? retryAfter : undefined,
    body,
  );
}

export interface ChatEndpoint {
  url: string;
  key: string;
}

// One chat completion asked to answer in JSON, and the text of its reply ("" when it has none).
// `body` is the rest of the request: model, messages, temperature and any provider's own options
export async function chatCompletion({ url, key }: ChatEndpoint, body: object, timeoutMs: number, what: string): Promise<string> {
  const res = await fetch(`${url.replace(/\/$/, "")}/chat/completions`, {
    method: "POST",
    headers: { Authorization: `Bearer ${key}`, "Content-Type": "application/json" },
    body: JSON.stringify({ ...body, response_format: { type: "json_object" } }),
    signal: AbortSignal.timeout(timeoutMs),
  });
  await failIfNotOk(res, what);
  const reply = (await res.json()) as { choices: { message: { content: string } }[] };
  return reply.choices[0]?.message.content ?? "";
}

// The JSON in a model's reply, skipping any <think> block or text around the object
export function extractJson(content: string): unknown {
  const text = content.replace(/<think>[\s\S]*?<\/think>/gi, "").trim();
  try {
    return JSON.parse(text);
  } catch {
    const start = text.indexOf("{");
    const end = text.lastIndexOf("}");
    if (start === -1 || end <= start) return null;
    try {
      return JSON.parse(text.slice(start, end + 1));
    } catch {
      return null;
    }
  }
}

export interface Exhausted {
  until: number; // epoch ms
}

// A rate limit shorter than this is waited out by the next call; a longer one is a used-up quota
const MAX_WAIT_S = 60;
const UNIT_S: Record<string, number> = { h: 3600, m: 60, s: 1, ms: 0.001 };

// "Please try again in 1h2m3.5s", for a reply without a Retry-After header
function tryAgainInS(body: string): number | null {
  const match = /try again in ((?:[\d.]+(?:ms|h|m|s))+)/i.exec(body);
  if (!match) return null;
  let seconds = 0;
  for (const [, value, unit] of match[1].matchAll(/([\d.]+)(ms|h|m|s)/g)) seconds += Number(value) * UNIT_S[unit];
  return seconds;
}

function readExhausted(error: unknown, now: number): Exhausted | null {
  if (!(error instanceof HttpError) || error.status !== 429) return null;
  const waitS = error.retryAfterS ?? tryAgainInS(error.body) ?? MAX_WAIT_S;
  return { until: now + waitS * 1000 };
}

// Groq counts its free quotas per model. Models go best first: `call` gets the best one with quota
// left, and `whenOut` decides what happens once every model is used up (it rejects to give up)
export function createQuota<M extends string>(models: readonly M[], now = Date.now) {
  const blocked = new Map<M, Exhausted>();

  return async function withQuota<T>(call: (model: M) => Promise<T>, whenOut: (soonest: Exhausted) => Promise<unknown>): Promise<T> {
    for (;;) {
      const model = models.find((m) => (blocked.get(m)?.until ?? 0) <= now());
      if (!model) {
        await whenOut([...blocked.values()].reduce((a, b) => (b.until < a.until ? b : a)));
        continue;
      }
      try {
        return await call(model);
      } catch (error) {
        const exhausted = readExhausted(error, now());
        if (!exhausted) throw error;
        blocked.set(model, exhausted);
      }
    }
  };
}
