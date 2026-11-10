// Schema changes, applied in order at startup and never edited once shipped: add a new entry instead.
// Rings, presence and who sits in which room stay in memory (rooms.ts); only what must outlive a deploy is here.
export const MIGRATIONS: string[] = [
  `
  CREATE TABLE users (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Stored lowercased; the check repeats USERNAME_RE so a bug in the server can't write anything else
    username text NOT NULL UNIQUE CHECK (username ~ '^[a-z0-9_]{3,20}$'),
    password_hash text NOT NULL,
    recovery_hash text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
  );

  -- Only a hash of the token: a leaked table can't be used to sign in.
  CREATE TABLE sessions (
    token_hash bytea PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    client_kind text NOT NULL CHECK (client_kind IN ('web', 'native')),
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL
  );
  CREATE INDEX sessions_user ON sessions (user_id);

  -- created_by is null for codes made with scripts/invite.ts.
  CREATE TABLE invites (
    code_hash bytea PRIMARY KEY,
    created_by uuid REFERENCES users ON DELETE CASCADE,
    used_by uuid REFERENCES users ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    used_at timestamptz
  );
  CREATE INDEX invites_unused ON invites (created_by) WHERE used_at IS NULL;

  -- One row per pair, the smaller id first, so a request and its reverse can't both exist.
  CREATE TABLE friendships (
    user_a uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    user_b uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    status text NOT NULL CHECK (status IN ('pending', 'accepted')),
    requested_by uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_a, user_b),
    CHECK (user_a < user_b),
    CHECK (requested_by IN (user_a, user_b))
  );
  CREATE INDEX friendships_b ON friendships (user_b);

  CREATE TABLE blocks (
    blocker uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    blocked uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (blocker, blocked)
  );

  CREATE TABLE missed_calls (
    id bigserial PRIMARY KEY,
    callee uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    caller uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    at timestamptz NOT NULL DEFAULT now(),
    seen_at timestamptz
  );
  CREATE INDEX missed_calls_unseen ON missed_calls (callee) WHERE seen_at IS NULL;
  `,
];
