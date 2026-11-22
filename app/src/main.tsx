import "@fontsource-variable/sora";
import "@fontsource-variable/jetbrains-mono";
import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/components.css";
import "./styles/app.css";
import "./styles/views.css";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { Overlay } from "./Overlay";

const view = new URLSearchParams(location.search).get("view");

createRoot(document.getElementById("root")!).render(<StrictMode>{view === "overlay" ? <Overlay /> : <App />}</StrictMode>);
