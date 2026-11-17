// Names the web app's pages share.
import type { SignInProvider } from "../api/skald.schemas.ts";

/** The Account page's path: a link or a sign-in from it returns to it. */
export const accountPath = "/profile";

/** The providers' names for people. */
export const providerNames: Record<SignInProvider, string> = {
  github: "GitHub",
  google: "Google",
};
