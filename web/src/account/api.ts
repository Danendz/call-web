import type { ApiError, ApiErrorCode } from "../../../shared/api";

/** A refused /api request, with the server's reason. */
export class ApiFailure extends Error {
  constructor(readonly code: ApiErrorCode) {
    super(code);
  }
}

/** Calls an /api endpoint: POST with a JSON body when given one, else GET. Throws ApiFailure when refused. */
export async function api<T = void>(path: string, body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await fetch(path, {
      method: body === undefined ? "GET" : "POST",
      headers: body === undefined ? {} : { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
      credentials: "same-origin",
    });
  } catch {
    throw new ApiFailure("unavailable");
  }
  const data: unknown = res.status === 204 ? undefined : await res.json().catch(() => undefined);
  if (!res.ok) {
    const code = (data as ApiError | undefined)?.error;
    throw new ApiFailure(typeof code === "string" ? code : "unavailable");
  }
  return data as T;
}
