// Candidate TURN relay locations for /nettest. Each URL is a small response on a single (non-CDN) server in
// that city, so a keep-alive HTTPS request takes about one network round trip.

export interface NetTestTarget {
  id: string;
  city: string;
  url: string;
  note?: string;
}

export const NET_TEST_TARGETS: NetTestTarget[] = [
  { id: "tas", city: "Tashkent", url: "https://speedtest.uz/favicon.ico", note: "≈ direct route" },
  { id: "ala", city: "Almaty", url: "https://www.ps.kz/favicon.ico" },
  { id: "mow", city: "Moscow", url: "https://storage.yandexcloud.net/" },
  { id: "fra", city: "Frankfurt", url: "https://fra-de-ping.vultr.com/rtt-probe" },
  { id: "hkg", city: "Hong Kong", url: "https://oss-cn-hongkong.aliyuncs.com/" },
  { id: "sgp", city: "Singapore", url: "https://sgp-ping.vultr.com/rtt-probe", note: "current relay" },
  { id: "tyo", city: "Tokyo", url: "https://hnd-jp-ping.vultr.com/rtt-probe" },
  { id: "sel", city: "Seoul", url: "https://sel-kor-ping.vultr.com/rtt-probe" },
];

export type NetTestSide = "uz" | "cn";
export const NET_TEST_SIDES: NetTestSide[] = ["uz", "cn"];

export interface NetTestResult {
  id: string;
  minMs: number | null; // null = unreachable
  medianMs: number | null;
  failed: number;
}

export interface NetTestReport {
  side: NetTestSide;
  at: number;
  results: NetTestResult[];
}

export type NetTestBoard = Partial<Record<NetTestSide, NetTestReport>>;
