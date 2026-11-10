// Prints a new invite code, for the first account (every later one can come from a user's own codes).
// Run with the database's URL: `DATABASE_URL=… npm run invite`. On Railway, the Postgres service's public URL.

import { createInvite } from "../server/auth/accounts.js";
import { migrate, sql } from "../server/db/index.js";

if (!sql) {
  console.error("DATABASE_URL is not set");
  process.exit(1);
}
await migrate();
const invite = await createInvite(null);
console.log(`${invite!.code}  (valid until ${new Date(invite!.expiresAt).toISOString()})`);
await sql.end();
