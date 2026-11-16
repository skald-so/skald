import {
  getGetSessionResponseMock,
  getGetSignInProvidersResponseMock,
} from "../api/skald.msw.ts";
import { Cabinet } from "../app/Cabinet.tsx";
import { Home } from "../app/Home.tsx";
import { SignInPage } from "../app/SignInPage.tsx";

// The web app's screens with fixture answers of the generated mocks.

const providers = getGetSignInProvidersResponseMock({
  providers: ["github", "google"],
  sign_up: "open",
});

const session = getGetSessionResponseMock({
  account: "01968a2e-0000-7000-8000-000000000001",
  username: "ana",
  github_login: "ana",
  sign_in_method: "github",
});

/** The sign-in page of 03-sign-in.png, on a server with both providers and open sign-up. */
export function KitSignIn() {
  return <SignInPage providers={providers} returnTo="/" />;
}

/**
 * The cabinet of an account signed up on the web, with no space yet; at phone width with the side
 * menu open, which says how to make one.
 */
export function KitAccount() {
  return (
    <Cabinet session={session} spaces={[]} menuOpen>
      <Home session={session} />
    </Cabinet>
  );
}
