import { createContext } from "react";
import type { SignInProvider } from "../api/skald.schemas.ts";
import { providerNames } from "./names.ts";

export type MachineSignInState =
  "off" | "available" | { linkedProviders: string };

/** The machine sign-in instructions that apply to the current web session. */
export function machineSignIn(
  providers: readonly SignInProvider[],
  current?: SignInProvider,
): MachineSignInState {
  const devices = providers.filter(
    (provider) => provider === "github" || provider === "google",
  );
  if (devices.length === 0) return "off";
  if (current !== "corporate") return "available";
  return {
    linkedProviders: devices
      .map((provider) => providerNames[provider])
      .join(" or "),
  };
}

export const MachineSignIn = createContext<MachineSignInState>("available");
