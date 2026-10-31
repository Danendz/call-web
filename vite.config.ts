import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  root: "web",
  plugins: [react()],
  build: {
    outDir: "../dist/web",
    emptyOutDir: true,
  },
  server: {
    port: 5173,
    host: true,
    proxy: {
      "/ws": { target: "ws://localhost:8080", ws: true },
      "/api": "http://localhost:8080",
    },
  },
});
