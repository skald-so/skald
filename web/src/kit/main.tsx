import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { KitPage, type Theme } from "./KitPage.tsx";
import "../theme/index.ts";

// ?theme=light or ?theme=dark forces a theme; otherwise it follows the system setting.
const param = new URLSearchParams(location.search).get("theme");
const theme: Theme | undefined =
  param === "light" || param === "dark" ? param : undefined;
if (theme) document.documentElement.dataset.theme = theme;

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <KitPage theme={theme} />
  </StrictMode>,
);
