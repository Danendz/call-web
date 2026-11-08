// Writes shared/protocol.fixtures.ts to shared/fixtures.json for the native client's protocol test.
// `--check` fails instead of writing when the file is out of date, for CI.

import { readFileSync, writeFileSync } from "node:fs";
import * as fixtures from "../shared/protocol.fixtures.js";

const path = new URL("../shared/fixtures.json", import.meta.url);
const json = JSON.stringify(fixtures, null, 2) + "\n";

if (process.argv.includes("--check")) {
  if (readFileSync(path, "utf8") !== json) {
    console.error("shared/fixtures.json is out of date: run `npm run fixtures`");
    process.exit(1);
  }
} else {
  writeFileSync(path, json);
}
