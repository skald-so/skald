import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import { UsageCard } from "./UsageCard.tsx";
import styles from "./UsageCard.module.css";

afterEach(cleanup);

/** The bar named `name`: its value, maximum, spoken value and filled width. */
function bar(name: string) {
  const meter = screen.getByRole("meter", { name });
  const fill = meter.firstElementChild as HTMLElement;
  return {
    now: meter.getAttribute("aria-valuenow"),
    max: meter.getAttribute("aria-valuemax"),
    text: meter.getAttribute("aria-valuetext"),
    width: fill.style.width,
    full: fill.classList.contains(styles.full!),
  };
}

test("a count under its limit shows it of the limit and fills its share of the bar", () => {
  const { container } = render(
    <UsageCard label="Devices" count={4} limit={10} />,
  );

  expect(container.textContent).toBe("Devices4 of 10");
  expect(bar("Devices")).toEqual({
    now: "4",
    max: "10",
    text: "4 of 10",
    width: "40%",
    full: false,
  });
});

test("a small count still shows on the bar, and large numbers have separators", () => {
  const { container } = render(
    <UsageCard label="Messages this month" count={12_480} limit={10_000_000} />,
  );

  expect(container.textContent).toBe("Messages this month12,480 of 10,000,000");
  expect(bar("Messages this month")).toMatchObject({
    now: "12480",
    max: "10000000",
    text: "12,480 of 10,000,000",
    width: "1%",
  });
});

test("an empty count leaves the bar empty", () => {
  render(<UsageCard label="Spaces" count={0} limit={2} />);

  expect(bar("Spaces")).toMatchObject({ width: "0%", full: false });
});

test("a count at its limit fills the bar in the warning color", () => {
  const { container } = render(
    <UsageCard label="Spaces" count={2} limit={2} />,
  );

  expect(container.textContent).toBe("Spaces2 of 2");
  expect(bar("Spaces")).toEqual({
    now: "2",
    max: "2",
    text: "2 of 2",
    width: "100%",
    full: true,
  });
});

test("a count without a limit says so and has no bar", () => {
  const { container } = render(<UsageCard label="Devices" count={37} />);

  expect(container.textContent).toBe("Devices37 No limit");
  expect(screen.queryByRole("meter")).toBeNull();
  expect(screen.queryByRole("progressbar")).toBeNull();
});
