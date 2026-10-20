import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles/tokens.css";
import "./styles/app.css";
import "./styles/conversation.css";
import "./styles/inspector.css";
import "./styles/pages.css";
import "./styles/todo.css";
import "./styles/terminal.css";
import "./styles/highlight.css";

const root = document.getElementById("root");
if (!root) throw new Error("#root is missing from index.html");

// The WebView default context menu (Back / Reload / Inspect Element) is
// browser chrome, not app UI — suppress it so right-click never shows it.
document.addEventListener("contextmenu", (event) => event.preventDefault());

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
