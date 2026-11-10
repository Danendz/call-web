// /api/friends/*: requests, unfriending and blocking. The new lists reach both sides over /ws/user.

import type { IncomingMessage, ServerResponse } from "node:http";
import { USERNAME_RE } from "../../shared/api.js";
import type { SessionUser } from "../auth/sessions.js";
import { HttpError, readJson, str } from "../http.js";
import { rateLimit } from "../limits.js";
import type { Invitee } from "../rooms.js";
import { refresh } from "./hub.js";
import * as store from "./store.js";

// A request is spam when there are many; accepting and removing are clicks on your own list.
const requestsPerUser = rateLimit(20, 60 * 60_000);
const changesPerUser = rateLimit(60, 60_000);

type Action = (me: string, target: Invitee) => Promise<store.FriendResult | void>;

const actions: Record<string, Action> = {
  request: store.request,
  accept: store.accept,
  remove: store.remove,
  block: store.block,
  unblock: store.unblock,
};

/** Handles POST /api/friends/<action> with body { username }. Answers the same whether or not the user exists. */
export async function friendsRoute(req: IncomingMessage, res: ServerResponse, user: SessionUser, action: string): Promise<void> {
  // hasOwn: "constructor" and the like are on every object
  const run = Object.hasOwn(actions, action) ? actions[action] : undefined;
  if (!run) throw new HttpError(404, "not-found");
  const body = await readJson(req);
  const username = str(body, "username", 64).trim().toLowerCase();
  if (!(action === "request" ? requestsPerUser(user.userId) : changesPerUser(user.userId))) throw new HttpError(429, "rate-limited");
  if (!USERNAME_RE.test(username)) throw new HttpError(400, "invalid-username");

  const target = await store.userByName(username);
  if (target) {
    if ((await run(user.userId, target)) === "too-many") throw new HttpError(409, "too-many");
    refresh([user.userId, target.id]);
  }
  res.writeHead(204).end();
}
