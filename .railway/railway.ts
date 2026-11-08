import { bucket, defineRailway, github, preserve, project, ref, service } from "railway/iac";

export default defineRailway(() => {
  // Native app releases, uploaded by CI (native-release.yml) and served at /updates/* by the web service. Private:
  // Railway buckets have no public access, and the bucket's own host isn't known to be reachable from China.
  const releases = bucket("releases", { region: "sin" });

  const web = service("web", {
    // Every push to main deploys; without this, `railway config apply` would disconnect the repo
    source: github("Danendz/call-web", { branch: "main" }),
    build: {
      buildCommand: "npm run build",
      // Only what the server and web client are built from: merges that touch only native/ or docs don't redeploy
      watchPatterns: ["server/**", "web/**", "shared/**", "package.json", "package-lock.json", "tsconfig*.json", "vite.config.ts"],
    },
    start: "npm start",
    healthcheck: "/healthz",
    // Singapore: best routes to mainland China, and next to the TURN relay on Vultr SG.
    replicas: { "asia-southeast1-eqsg3a": 1 },
    env: {
      TURN_HOST: "turn.danendz.com",
      TURN_SECRET: preserve(), // must equal coturn's static-auth-secret; set with `railway variable set`
      // Speech-to-chat and translation keys, set with `railway variable set`
      GROQ_API_KEY: preserve(),
      LLM_BASE_URL: preserve(),
      LLM_API_KEY: preserve(),
      LLM_MODEL: preserve(),
      RELEASES_ENDPOINT: ref(releases, "ENDPOINT"),
      RELEASES_BUCKET: ref(releases, "BUCKET"),
      RELEASES_REGION: ref(releases, "REGION"),
      RELEASES_KEY_ID: ref(releases, "ACCESS_KEY_ID"),
      RELEASES_SECRET: ref(releases, "SECRET_ACCESS_KEY"),
    },
  });

  return project("call-danya", { resources: [releases, web] });
});
