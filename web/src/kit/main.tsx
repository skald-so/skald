import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import {
  KitAccess,
  KitAccount,
  KitAccountPage,
  KitMachines,
  KitMembers,
  KitSignIn,
  KitUsage,
} from "./KitApp.tsx";
import { KitCabinet } from "./KitCabinet.tsx";
import { KitPage, type Theme } from "./KitPage.tsx";
import "../theme/index.ts";

// ?theme=light or ?theme=dark forces a theme; otherwise it follows the system setting.
// ?screen=cabinet shows the cabinet around Machines; ?screen=cabinet-menu opens its side menu at
// phone width; ?screen=sign-in shows the web app's sign-in page, ?screen=account the cabinet of an
// account with no space, its side menu open at phone width, ?screen=machines the Machines section,
// ?screen=members the Members section, ?screen=access the Access section,
// ?screen=access-confirm its question before "Sign out everywhere else", ?screen=account-page the
// Account page, ?screen=account-page-confirm its question before removing this session's sign-in
// method, ?screen=account-page-blocked the Account page with a space blocking the deletion,
// ?screen=account-page-delete its question before deleting the account and ?screen=usage the Usage
// section. Without it, the kit page.
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
    ) : screen === "machines" ? (
      <KitMachines />
    ) : screen === "members" ? (
      <KitMembers />
    ) : screen === "access" || screen === "access-confirm" ? (
      <KitAccess asking={screen === "access-confirm"} />
    ) : screen === "account-page" ? (
      <KitAccountPage />
    ) : screen === "account-page-confirm" ? (
      <KitAccountPage view="confirm" />
    ) : screen === "account-page-blocked" ? (
      <KitAccountPage view="blocked" />
    ) : screen === "account-page-delete" ? (
      <KitAccountPage view="delete" />
    ) : screen === "usage" ? (
      <KitUsage />
    ) : (
      <KitPage theme={theme} />
    )}
  </StrictMode>,
);
