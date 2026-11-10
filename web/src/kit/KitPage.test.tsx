import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { KitPage, type Theme } from "./KitPage.tsx";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

test("shows the color tokens and fonts without network requests", () => {
  const fetch = vi.spyOn(globalThis, "fetch");
  render(<KitPage />);

  const colors = within(screen.getByRole("region", { name: "Colors" }));
  expect(colors.getAllByRole("listitem")).toHaveLength(9);
  const fonts = within(screen.getByRole("region", { name: "Fonts" }));
  expect(
    fonts.getAllByRole("listitem").map((font) => font.firstChild?.textContent),
  ).toEqual([
    "Inter · Text",
    "Space Grotesk · Headings and numbers",
    "JetBrains Mono · Code, keys and IDs",
  ]);
  expect(fetch).not.toHaveBeenCalled();
});

test.each<[Theme | undefined, string]>([
  [undefined, "System"],
  ["light", "Light"],
  ["dark", "Dark"],
])("with the theme %s, the %s theme link is current", (theme, current) => {
  render(<KitPage theme={theme} />);

  const links = within(
    screen.getByRole("navigation", { name: "Theme" }),
  ).getAllByRole("link");
  expect(
    links.map((link) => [
      link.textContent,
      link.getAttribute("href"),
      link.getAttribute("aria-current"),
    ]),
  ).toEqual(
    [
      ["System", "kit.html"],
      ["Light", "kit.html?theme=light"],
      ["Dark", "kit.html?theme=dark"],
    ].map(([label, href]) => [label, href, label === current ? "page" : null]),
  );
});
