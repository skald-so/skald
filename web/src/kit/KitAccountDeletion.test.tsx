// The kit page's Account views show the account's deletion part: one with no space blocking it,
// whose Delete shows, and one with a blocking space, named with the command that hands over admin
// rights, `skald hand-over-admin <space> …`, and no Delete. The views are the kit page's screens of
// the Account page, those of `kit-references.ts` whose name starts with `account-page`, each
// rendered as the kit page renders `?screen=<name>`; so their reference screenshots show the part.
import { act, cleanup, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { screens } from "../../kit-references.ts";

afterEach(() => {
  cleanup();
  document.body.innerHTML = "";
});

/** The Account page's screens of the kit page. */
const accountScreens = screens
  .map(({ screen }) => screen as string)
  .filter((name) => name.startsWith("account-page"));

/** What the kit page shows at `?screen=<name>`: Delete outside a dialog, and the page's text. */
async function shown(name: string): Promise<{ deletes: number; text: string }> {
  document.body.innerHTML = '<div id="root"></div>';
  window.history.replaceState(null, "", `/kit.html?screen=${name}`);
  vi.resetModules();
  await act(async () => {
    await import("./main.tsx");
  });
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 50));
  });
  const deletes = screen
    .queryAllByRole("button", { name: /^delete\b/i })
    .filter((button) => !button.closest("[role=dialog], [role=alertdialog]"));
  return { deletes: deletes.length, text: document.body.textContent ?? "" };
}

test("The kit page's Account views show the deletion part without and with a blocking space", async () => {
  expect(accountScreens.length).toBeGreaterThan(0);
  const views = [];
  for (const name of accountScreens)
    views.push({ name, ...(await shown(name)) });

  const blocking = /skald hand-over-admin\s+["']?[a-z0-9][\w-]*/;
  const free = views.filter(
    (view) => view.deletes === 1 && !blocking.test(view.text),
  );
  const blocked = views.filter(
    (view) => view.deletes === 0 && blocking.test(view.text),
  );
  const seen = views.map(({ name, deletes, text }) => ({
    name,
    deletes,
    blocking: blocking.test(text),
  }));
  expect(free, JSON.stringify(seen)).not.toHaveLength(0);
  expect(blocked, JSON.stringify(seen)).not.toHaveLength(0);
});
