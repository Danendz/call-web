import { defineRailway, preserve, project, service } from "railway/iac";

export default defineRailway(() => {
  const web = service("web", {
    build: "npm run build",
    start: "npm start",
    healthcheck: "/healthz",
    // Singapore: best routes to mainland China, and next to the TURN relay on Vultr SG.
    replicas: { "asia-southeast1-eqsg3a": 1 },
    env: {
      TURN_HOST: "turn.danendz.com",
      TURN_SECRET: preserve(), // must equal coturn's static-auth-secret; set with `railway variable set`
    },
  });

  return project("call-danya", { resources: [web] });
});
