import { createHmac } from "node:crypto";
import type { IceServerConfig } from "../shared/protocol.js";

const TURN_HOST = process.env.TURN_HOST; // e.g. turn.danendz.com
const TURN_SECRET = process.env.TURN_SECRET; // must equal coturn's static-auth-secret
const TURN_PORT = process.env.TURN_PORT ?? "3478";
const TURNS_PORT = process.env.TURNS_PORT ?? "5349";
const CREDENTIAL_TTL_S = 24 * 60 * 60;

/**
 * ICE servers for one room member. Uses coturn's "TURN REST API" scheme
 * (use-auth-secret): username = "<expiry>:<label>", credential = base64(HMAC-SHA1(secret, username)).
 * Credentials are only handed out over the room socket, so only room members get relay access.
 */
export function iceServersFor(roomId: string): IceServerConfig[] {
  if (!TURN_HOST) return [];

  const stun: IceServerConfig = { urls: [`stun:${TURN_HOST}:${TURN_PORT}`] };
  if (!TURN_SECRET) return [stun];

  const username = `${Math.floor(Date.now() / 1000) + CREDENTIAL_TTL_S}:${roomId}`;
  const credential = createHmac("sha1", TURN_SECRET).update(username).digest("base64");

  return [
    stun,
    {
      urls: [
        `turn:${TURN_HOST}:${TURN_PORT}?transport=udp`,
        `turn:${TURN_HOST}:${TURN_PORT}?transport=tcp`,
        `turns:${TURN_HOST}:${TURNS_PORT}?transport=tcp`,
      ],
      username,
      credential,
    },
  ];
}
