import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import type { SignInProviders } from "../api/skald.schemas.ts";
import { SignInPage } from "./SignInPage.tsx";

afterEach(cleanup);

const policy = "https://policies.example/skald/privacy?lang=en&v=2";

test.each([
  [["github", "google"], "open"],
  [["google"], "closed"],
  [[], "open"],
] as [SignInProviders["providers"], SignInProviders["sign_up"]][])(
  "with the server's privacy policy, the page links it: providers %j, sign-up %s",
  (names, signUp) => {
    render(
      <SignInPage
        providers={{
          providers: names,
          sign_up: signUp,
          privacy_policy_url: policy,
        }}
        returnTo="/"
      />,
    );
    const link = screen.getByRole("link", { name: "Privacy policy" });
    expect(link.getAttribute("href")).toBe(policy);
  },
);

test("without a privacy policy, the page has no link to one", () => {
  render(
    <SignInPage
      providers={{ providers: ["github", "google"], sign_up: "open" }}
      returnTo="/"
    />,
  );
  expect(screen.queryByRole("link", { name: /privacy/i })).toBeNull();
  expect(document.body.textContent).not.toMatch(/privacy/i);
  expect(screen.getAllByRole("link")).toHaveLength(2);
});
