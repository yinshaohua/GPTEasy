import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "./App";
import { recordFrontendFailure } from "./contracts/diagnostics";
import "./global.css";

window.addEventListener("error", () => {
  void recordFrontendFailure("unhandled_error").catch(() => undefined);
});
window.addEventListener("unhandledrejection", () => {
  void recordFrontendFailure("unhandled_rejection").catch(() => undefined);
});

const root = document.getElementById("root");

if (!root) {
  throw new Error("Missing application root");
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
