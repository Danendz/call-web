import postgres from "postgres";
import { MIGRATIONS } from "./migrations.js";

// Without DATABASE_URL accounts are off and only room links work, like the other optional features.
export const sql = process.env.DATABASE_URL
  ? postgres(process.env.DATABASE_URL, { max: 10, idle_timeout: 60, onnotice: () => {} })
  : null;

// Any number works, as long as nothing else takes the same advisory lock.
const MIGRATION_LOCK = 4_242_001;

/** Applies migrations not yet run. Throws on failure, so a bad deploy fails its healthcheck instead of serving. */
export async function migrate(): Promise<void> {
  if (!sql) return;
  await sql.begin(async (tx) => {
    // A second replica starting at the same time waits here instead of applying the same migration twice.
    await tx`SELECT pg_advisory_xact_lock(${MIGRATION_LOCK})`;
    await tx`CREATE TABLE IF NOT EXISTS schema_migrations (version int PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT now())`;
    const [{ done }] = await tx<{ done: number }[]>`SELECT count(*)::int AS done FROM schema_migrations`;
    for (let version = done; version < MIGRATIONS.length; version++) {
      await tx.unsafe(MIGRATIONS[version]);
      await tx`INSERT INTO schema_migrations (version) VALUES (${version})`;
      console.log(`applied migration ${version}`);
    }
  });
}
