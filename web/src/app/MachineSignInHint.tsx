import { useContext } from "react";
import { MachineSignIn } from "./machine-sign-in.ts";

export function MachineSignInHint({ purpose }: { purpose: string }) {
  const signIn = useContext(MachineSignIn);
  if (signIn === "off") return null;
  return (
    <>
      {" "}
      Sign in a machine with <code>skald</code> {purpose}.
      {signIn !== "available" && (
        <> You need a linked {signIn.linkedProviders} sign-in method.</>
      )}
    </>
  );
}
