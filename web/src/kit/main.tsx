import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { KitAccess, KitAccount, KitAccountPage, KitSignIn } from "./KitApp.tsx";
import { KitCabinet } from "./KitCabinet.tsx";
import { KitPage, type Theme } from "./KitPage.tsx";
import "../theme/index.ts";

// ?theme=light or ?theme=dark forces a theme; otherwise it follows the system setting.
// ?screen=cabinet shows the cabinet around Machines; ?screen=cabinet-menu opens its side menu at
// phone width; ?screen=sign-in shows the web app's sign-in page, ?screen=account the cabinet of an
// account with no space, its side menu open at phone width, ?screen=access the Access section and
// ?screen=access-confirm its question before "Sign out everywhere else", ?screen=account-page the
// Account page and ?screen=account-page-confirm its question before removing this session's sign-in
// method. Without it, the kit page.
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
    ) : screen === "sign-in" ? (
      <KitSignIn />
    ) : screen === "account" ? (
      <KitAccount />
    ) : screen === "access" || screen === "access-confirm" ? (
      <KitAccess asking={screen === "access-confirm"} />
    ) : screen === "account-page" || screen === "account-page-confirm" ? (
      <KitAccountPage asking={screen === "account-page-confirm"} />
    ) : (
      <KitPage theme={theme} />
    )}
  </StrictMode>,
);
