// Where lines end and when live text is decoded again: with a VAD and a decoder the test drives by hand.

import assert from "node:assert/strict";
import { test } from "node:test";
import { TRANSCRIBE_SAMPLE_RATE } from "../shared/protocol.js";
import { Utterance, type Decoder, type Vad } from "./speechLines.js";

const chunk = (s: number) => new Float32Array(Math.round(s * TRANSCRIBE_SAMPLE_RATE));

function fakeVad() {
  const vad = { speaking: false, ended: null as number | null, open: false };
  const api: Vad = {
    accept() {},
    ended() {
      const ended = vad.ended;
      vad.ended = null;
      return ended;
    },
    flush() {
      if (vad.open) vad.ended = Infinity;
      vad.open = false;
    },
    speaking: () => vad.speaking,
  };
  return { vad, api };
}

// Decodes come back only when the test says, as `<seconds>s`
function fakeDecoder() {
  const pending: { seconds: number; resolve: (text: string) => void }[] = [];
  const api: Decoder = {
    decode: (samples) =>
      new Promise((resolve) => pending.push({ seconds: samples.length / TRANSCRIBE_SAMPLE_RATE, resolve })),
    waiting: () => pending.length,
  };
  const next = async (text?: string) => {
    const d = pending.shift()!;
    d.resolve(text ?? `${d.seconds}s`);
    await new Promise((r) => setImmediate(r));
    return d.seconds;
  };
  return { pending, api, next };
}

function sink() {
  const out: string[] = [];
  return {
    out,
    api: {
      live: (id: string, text: string) => out.push(`live ${id} ${text}`),
      line: (text: string, liveId: string | undefined) => out.push(`line ${liveId ?? "-"} ${text}`),
    },
  };
}

test("each pause ends a line, posted in order with the live text it replaces", async () => {
  const { vad, api: vadApi } = fakeVad();
  const dec = fakeDecoder();
  const s = sink();
  let ids = 0;
  const u = new Utterance(vadApi, dec.api, true, s.api, () => `L${++ids}`);

  vad.speaking = true;
  u.audio(chunk(0.5));
  assert.equal(await dec.next(), 0.5); // live decode of the first line
  vad.speaking = false;
  vad.ended = Infinity;
  u.audio(chunk(0.1)); // the pause ends it
  vad.speaking = true;
  u.audio(chunk(0.5)); // the next line, no live decode yet: the first line's is waiting
  vad.open = true;
  u.end();

  assert.equal(dec.pending.length, 2);
  await dec.next();
  await dec.next("嗯。"); // a filler alone isn't posted
  assert.deepEqual(s.out, ["live L1 0.5s", "line L1 0.6s"]);
});

test("a line ends where speech did, and the next one keeps its start", async () => {
  const { vad, api: vadApi } = fakeVad();
  const dec = fakeDecoder();
  const s = sink();
  const u = new Utterance(vadApi, dec.api, false, s.api, () => "L");

  vad.speaking = true;
  u.audio(chunk(1));
  // The VAD sees the pause only once the next line has begun: speech ended 0.6 s in
  vad.ended = 0.6 * TRANSCRIBE_SAMPLE_RATE;
  u.audio(chunk(0.2));
  vad.open = true;
  u.end();

  // The first line keeps 0.3 s past its end; the other 0.3 s starts the next
  assert.equal(await dec.next(), 0.9);
  assert.equal(await dec.next(), 0.3);
});

test("live text isn't decoded again while the decoder is behind, and a stale result is dropped", async () => {
  const { vad, api: vadApi } = fakeVad();
  const dec = fakeDecoder();
  const s = sink();
  const u = new Utterance(vadApi, dec.api, true, s.api, () => "L1");

  vad.speaking = true;
  u.audio(chunk(0.5));
  u.audio(chunk(0.5)); // behind: skipped
  u.audio(chunk(0.5)); // still behind: skipped
  assert.equal(dec.pending.length, 1);
  await dec.next();
  u.audio(chunk(0.5)); // caught up: decodes the whole line so far
  assert.equal(dec.pending.length, 1);

  vad.ended = Infinity;
  u.audio(chunk(0.1)); // the line ends before its last live decode is back
  await dec.next("stale");
  await dec.next("done");
  assert.deepEqual(s.out, ["live L1 0.5s", "line L1 done"]);
});

test("an utterance with no speech in it posts nothing, and live text with no line is taken back", async () => {
  const { vad, api: vadApi } = fakeVad();
  const dec = fakeDecoder();
  const s = sink();
  const quiet = new Utterance(vadApi, dec.api, true, s.api, () => "Q");
  for (let i = 0; i < 20; i++) quiet.audio(chunk(0.5));
  quiet.end();
  assert.equal(dec.pending.length, 0);

  vad.speaking = true;
  const cancelled = new Utterance(vadApi, dec.api, true, s.api, () => "C");
  cancelled.audio(chunk(0.5));
  await dec.next("hi");
  cancelled.cancel();
  assert.deepEqual(s.out, ["live C hi", "live C "]);
});
