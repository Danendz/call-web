import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { NetTest } from "./nettest/NetTest";
import "./styles.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {location.pathname === "/nettest" ? <NetTest /> : <App />}
  </StrictMode>,
);
