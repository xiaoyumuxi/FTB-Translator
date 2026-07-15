import React from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { FrontendErrorBoundary } from "./components/FrontendErrorBoundary";
import "./styles.css";

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <FrontendErrorBoundary>
      <App />
    </FrontendErrorBoundary>
  </React.StrictMode>,
);
