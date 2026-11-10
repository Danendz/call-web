// Request and response bodies of the /api endpoints. Shared by server and web client.

export const USERNAME_RE = /^[a-z0-9_]{3,20}$/;
export const MIN_PASSWORD_LENGTH = 10;
export const MAX_PASSWORD_LENGTH = 200;
// Unused invite codes one account can hold at a time, and how long a code stays valid.
export const MAX_UNUSED_INVITES = 3;
export const INVITE_TTL_DAYS = 7;

// The web client signs in with a cookie; the native client keeps the token from the response and sends it as
// `Authorization: Bearer <token>`.
export type ClientKind = "web" | "native";

export interface RegisterRequest {
  username: string;
  password: string;
  invite: string;
  client: ClientKind;
}

export interface LoginRequest {
  username: string;
  password: string;
  client: ClientKind;
}

// Sets a new password with the recovery code; signs out every session and replaces the code.
export interface RecoverRequest {
  username: string;
  recoveryCode: string;
  password: string;
  client: ClientKind;
}

// Signs out every other session.
export interface ChangePasswordRequest {
  password: string;
  newPassword: string;
}

export interface DeleteAccountRequest {
  password: string;
}

export interface SessionResponse {
  username: string;
  // Only for the native client; the web client gets an HttpOnly cookie instead
  token?: string;
  // After register and recover only: shown once, never retrievable again
  recoveryCode?: string;
}

export interface MeResponse {
  username: string;
}

export interface InvitesResponse {
  unused: number;
  max: number;
}

// Shown once: only a hash of the code is stored.
export interface NewInviteResponse {
  code: string;
  expiresAt: number;
}

export type ApiErrorCode =
  | "bad-request"
  | "not-found"
  | "unauthorized"
  | "forbidden"
  | "rate-limited"
  | "unavailable"
  | "invalid-credentials"
  | "invalid-invite"
  | "invalid-recovery"
  | "username-taken"
  | "invalid-username"
  | "weak-password"
  | "too-many-invites"
  // Past the most pending friend requests, friends or blocks one account can have
  | "too-many";

export interface ApiError {
  error: ApiErrorCode;
}
