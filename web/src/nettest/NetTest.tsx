import { useEffect, useState } from "react";
import {
  NET_TEST_TARGETS,
  type NetTestBoard,
  type NetTestResult,
  type NetTestSide,
  type NetTestTarget,
} from "../../../shared/nettest";

const SAMPLES = 12;
const GAP_MS = 250;
const TIMEOUT_MS = 5_000;

const SIDE_LABEL: Record<NetTestSide, string> = { uz: "Uzbekistan", cn: "China 中国" };

/**
 * Times round trips from this browser to each candidate relay city. A relayed call's RTT is roughly
 * (your RTT to the relay) + (their RTT to the relay), so the best relay has the lowest sum across both sides.
 */
export function NetTest() {
  const [side, setSide] = useState<NetTestSide | null>(null);
  const [live, setLive] = useState<Record<string, NetTestResult>>({});
  const [running, setRunning] = useState(false);
  const [board, setBoard] = useState<NetTestBoard>({});

  useEffect(() => {
    const load = () =>
      fetch("/api/nettest", { cache: "no-store" })
        .then((r) => r.json())
        .then(setBoard)
        .catch(() => {});
    void load();
    const timer = setInterval(load, 5_000);
    return () => clearInterval(timer);
  }, []);

  const run = async (me: NetTestSide) => {
    setSide(me);
    setRunning(true);
    setLive({});
    const results = await Promise.all(
      NET_TEST_TARGETS.map((target) => measure(target, (r) => setLive((prev) => ({ ...prev, [target.id]: r })))),
    );
    const res = await fetch("/api/nettest", { method: "POST", body: JSON.stringify({ side: me, results }) }).catch(() => null);
    if (res?.ok) setBoard(await res.json());
    setRunning(false);
  };

  const sum = (id: string) => {
    const uz = board.uz?.results.find((r) => r.id === id)?.medianMs;
    const cn = board.cn?.results.find((r) => r.id === id)?.medianMs;
    return uz != null && cn != null ? uz + cn : null;
  };
  const best = NET_TEST_TARGETS.map((t) => sum(t.id))
    .filter((v): v is number => v !== null)
    .sort((a, b) => a - b)[0];

  return (
    <div className="nettest">
      <h1>Relay location test</h1>
      <p className="muted">
        Turn off any VPN, then tap where you are. Takes about 15 seconds.
        <br />
        请关闭VPN，然后点击你所在的位置。大约需要15秒。
      </p>
      <div className="nettest-sides">
        <button className="primary" disabled={running} onClick={() => run("uz")}>
          I'm in Uzbekistan
        </button>
        <button className="primary" disabled={running} onClick={() => run("cn")}>
          我在中国 / I'm in China
        </button>
      </div>

      <table>
        <thead>
          <tr>
            <th>City</th>
            {side && <th>Now ({side === "uz" ? "UZ" : "CN"})</th>}
            <th>Uzbekistan</th>
            <th>China</th>
            <th>Call RTT ≈</th>
          </tr>
        </thead>
        <tbody>
          {NET_TEST_TARGETS.map((t) => {
            const total = sum(t.id);
            return (
              <tr key={t.id} className={total !== null && total === best ? "best" : ""}>
                <td>
                  {t.city}
                  {t.note && <small className="muted"> {t.note}</small>}
                </td>
                {side && <td>{fmtResult(live[t.id], running)}</td>}
                <td>{fmtResult(board.uz?.results.find((r) => r.id === t.id))}</td>
                <td>{fmtResult(board.cn?.results.find((r) => r.id === t.id))}</td>
                <td>{total === null ? "—" : `${total} ms`}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <p className="muted small">
        Median round trip in ms (best in brackets). A median far above the best means packet loss on that route.
        Tested: {SIDE_LABEL.uz} {fmtAt(board.uz?.at)} · {SIDE_LABEL.cn}{" "}
        {fmtAt(board.cn?.at)}
      </p>
    </div>
  );
}

async function measure(target: NetTestTarget, onProgress: (r: NetTestResult) => void): Promise<NetTestResult> {
  const times: number[] = [];
  let failed = 0;
  // First request opens DNS + TCP + TLS; later ones reuse the connection and cost about one round trip.
  for (let i = 0; i <= SAMPLES; i++) {
    const started = performance.now();
    try {
      await fetch(`${target.url}?nt=${Date.now()}-${i}`, {
        mode: "no-cors",
        cache: "no-store",
        credentials: "omit",
        signal: AbortSignal.timeout(TIMEOUT_MS),
      });
      if (i > 0) times.push(performance.now() - started);
    } catch {
      failed++;
    }
    onProgress(summarize(target.id, times, failed));
    await new Promise((r) => setTimeout(r, GAP_MS));
  }
  return summarize(target.id, times, failed);
}

function summarize(id: string, times: number[], failed: number): NetTestResult {
  if (times.length === 0) return { id, minMs: null, medianMs: null, failed };
  const sorted = [...times].sort((a, b) => a - b);
  return {
    id,
    minMs: Math.round(sorted[0]),
    medianMs: Math.round(sorted[Math.floor(sorted.length / 2)]),
    failed,
  };
}

function fmtResult(r: NetTestResult | undefined, running = false): string {
  if (!r) return running ? "…" : "—";
  if (r.medianMs === null) return r.failed ? "unreachable" : "…";
  return `${r.medianMs} (${r.minMs})${r.failed ? ` ⚠${r.failed}` : ""}`;
}

function fmtAt(at: number | undefined): string {
  return at ? new Date(at).toLocaleString() : "not yet";
}
