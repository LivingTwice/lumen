import "@fontsource-variable/geist";
import "@fontsource-variable/literata";
import "@fontsource-variable/literata/wght-italic.css";
import "@fontsource-variable/newsreader";
import "@fontsource-variable/newsreader/wght-italic.css";
import "./styles/app.css";
import "./styles/views.css";
import "./styles/reader.css";
import "./styles/onboarding.css";
import "./styles/chat.css";

import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { api, initApi, isTauri } from "./lib/api";
import { useApp } from "./lib/store";

initApi().then(() => {
  // accès de test dans l'aperçu navigateur (jamais dans l'application)
  if (import.meta.env.DEV && !isTauri) (window as unknown as Record<string, unknown>).__lumen = { api, useApp };
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
});
