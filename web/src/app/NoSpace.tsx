import { MachineSignInHint } from "./MachineSignInHint.tsx";

/** What a page says while the account has no space. */
export function NoSpace() {
  return (
    <>
      No space yet.
      <MachineSignInHint purpose="to make your personal space" />
    </>
  );
}
