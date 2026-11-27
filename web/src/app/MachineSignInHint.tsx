import { useContext } from "react";
import { MachineSignIn } from "./machine-sign-in.ts";

export function MachineSignInHint({ purpose }: { purpose: string }) {
  if (!useContext(MachineSignIn)) return null;
  return (
    <>
      {" "}
      Sign in a machine with <code>skald</code> {purpose}.
    </>
  );
}
