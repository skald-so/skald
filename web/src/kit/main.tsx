import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { KitCabinet } from "./KitCabinet.tsx";
import { KitPage, type Theme } from "./KitPage.tsx";
import "../theme/index.ts";

// ?theme=light or ?theme=dark forces a theme; otherwise it follows the system setting.
// ?screen=cabinet shows the cabinet around Machines; ?screen=cabinet-menu opens its side menu at
// phone width. Without it, the kit page.
const params = new URLSearchParams(location.search);
const param = params.get("theme");
const theme: Theme | undefined =
  param === "light" || param === "dark" ? param : undefined;
if (theme) document.documentElement.dataset.theme = theme;
const screen = params.get("screen");

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {screen === "cabinet" || screen === "cabinet-menu" ? (
      <KitCabinet menuOpen={screen === "cabinet-menu"} />
    ) : (
      <KitPage theme={theme} />
    )}
  </StrictMode>,
);
