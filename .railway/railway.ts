import { bucket, defineRailway, github, postgres, preserve, project, ref, service, volume } from "railway/iac";

export default defineRailway(() => {
  // Native app releases, uploaded by CI (native-release.yml) and served at /updates/* by the web service. Private:
  // Railway buckets have no public access, and the bucket's own host isn't known to be reachable from China.
  const releases = bucket("releases", { region: "sin" });

  // Accounts, sessions, invites and friends. Next to the web service, which is its only client.
  const db = postgres("db", { region: "asia-southeast1-eqsg3a" });

  // Speech recognition (server/speechService.ts), apart from the web service so its ~1 GB of models is held only
  // while someone uses speech: it sleeps when idle, and the web service's first connection wakes it. No public
  // domain: it's reached only over the private network.
  const speech = service("speech", {
    source: github("Danendz/call-web", { branch: "main" }),
    build: {
      buildCommand: "npm run build:speech",
      watchPatterns: ["server/speech*.ts", "server/updates.ts", "server/limits.ts", "shared/**", "package.json", "package-lock.json", "tsconfig*.json"],
    },
    start: "npm run start:speech",
    healthcheck: "/healthz",
    replicas: { "asia-southeast1-eqsg3a": 1 },
    deploy: { sleepApplication: true },
    // The models, kept across deploys and wake-ups instead of fetched again each time
    volumeMounts: { "/models": volume("speech-models", { sizeMB: 1024, region: "asia-southeast1-eqsg3a" }) },
    env: {
      PORT: "8090",
      MODELS_DIR: "/models",
      SPEECH_SECRET: preserve(), // any long random string; set with `railway variable set`
      RELEASES_ENDPOINT: ref(releases, "ENDPOINT"),
      RELEASES_BUCKET: ref(releases, "BUCKET"),
      RELEASES_REGION: ref(releases, "REGION"),
      RELEASES_KEY_ID: ref(releases, "ACCESS_KEY_ID"),
      RELEASES_SECRET: ref(releases, "SECRET_ACCESS_KEY"),
    },
  });

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
    // Singapore: closest Railway region to mainland China. Only signaling runs here; media goes direct or via the TURN relay.
    replicas: { "asia-southeast1-eqsg3a": 1 },
    env: {
      // coturn on Alibaba Cloud Hong Kong (China-optimized BGP). Vultr SG lost 10-30% of packets on the Singtel link.
      TURN_HOST: "turn-hk.danendz.com",
      TURN_SECRET: preserve(), // must equal coturn's static-auth-secret; set with `railway variable set`
      SPEECH_HOST: ref(speech, "RAILWAY_PRIVATE_DOMAIN"),
      SPEECH_PORT: ref(speech, "PORT"),
      SPEECH_SECRET: ref(speech, "SPEECH_SECRET"),
      // Translation keys, set with `railway variable set`
      GROQ_API_KEY: preserve(),
      LLM_BASE_URL: preserve(),
      LLM_API_KEY: preserve(),
      LLM_MODEL: preserve(),
      DATABASE_URL: ref(db, "DATABASE_URL"),
      RELEASES_ENDPOINT: ref(releases, "ENDPOINT"),
      RELEASES_BUCKET: ref(releases, "BUCKET"),
      RELEASES_REGION: ref(releases, "REGION"),
      RELEASES_KEY_ID: ref(releases, "ACCESS_KEY_ID"),
      RELEASES_SECRET: ref(releases, "SECRET_ACCESS_KEY"),
    },
  });

  return project("call-danya", { resources: [releases, db, speech, web] });
});
