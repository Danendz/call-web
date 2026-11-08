import { defineRailway, preserve, project, service } from "railway/iac";

export default defineRailway(() => {
  const web = service("web", {
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
    },
  });

  return project("call-danya", { resources: [web] });
});
