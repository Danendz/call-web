import { useSyncExternalStore } from "react";
import type { MeResponse, SessionResponse } from "../../../shared/api";
import { ApiFailure, api } from "./api";

// Who is signed in. The session itself is an HttpOnly cookie the page can't read; /api/me says whose it is.

export type Session =
  | { status: "loading" }
  // accountsOn: false when the server has no database, so there's nothing to sign in to
  | { status: "signed-out"; accountsOn: boolean }
  // recoveryCode: new after register or recover, shown once before anything else
  | { status: "signed-in"; username: string; recoveryCode?: string };

let session: Session = { status: "loading" };
const listeners = new Set<() => void>();

function set(next: Session): void {
  session = next;
  listeners.forEach((l) => l());
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useSession(): Session {
  return useSyncExternalStore(subscribe, () => session);
}

export async function loadSession(): Promise<void> {
  try {
    const me = await api<MeResponse>("/api/me");
    set({ status: "signed-in", username: me.username });
  } catch (err) {
    set({ status: "signed-out", accountsOn: !(err instanceof ApiFailure && err.code === "unavailable") });
  }
}

/** Sign in, register or recover: each answers with the session. */
export async function startSession(path: string, body: Record<string, string>): Promise<void> {
  const res = await api<SessionResponse>(path, { ...body, client: "web" });
  set({ status: "signed-in", username: res.username, ...(res.recoveryCode && { recoveryCode: res.recoveryCode }) });
}

/** The new recovery code was saved: stop showing it. */
export function recoveryCodeSaved(): void {
  if (session.status === "signed-in") set({ status: "signed-in", username: session.username });
}

export async function signOut(): Promise<void> {
  await api("/api/auth/logout", {}).catch(() => {});
  set({ status: "signed-out", accountsOn: true });
}

export function signedOut(): void {
  set({ status: "signed-out", accountsOn: true });
}
