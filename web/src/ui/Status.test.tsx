import { cleanup, render } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import a11y from "./a11y.module.css";
import { CountBadge, StatusDot, StatusLabel, type Tone } from "./Status.tsx";

afterEach(cleanup);

const tones: Tone[] = ["ok", "warning", "danger", "accent", "muted"];

test.each(tones)(
  "in the %s tone, the label and the dot read as their text alone",
  (tone) => {
    const { container } = render(
      <>
        <StatusLabel tone={tone}>Flagged</StatusLabel>
        <StatusDot tone={tone}>Online</StatusDot>
      </>,
    );

    const [label, dot] = [...container.children] as HTMLElement[];
    expect(label!.textContent).toBe("Flagged");
    expect(dot!.textContent).toBe("Online");
    // The dot is only color: a screen reader skips it.
    const marks = [...dot!.children];
    expect(marks.map((mark) => mark.getAttribute("aria-hidden"))).toEqual([
      "true",
    ]);
  },
);

test("a count badge shows the number and reads it with its label", () => {
  const { container } = render(
    <CountBadge tone="warning" count={2} label="in dead letters" />,
  );

  const badge = container.firstElementChild!;
  expect(badge.textContent).toBe("2 in dead letters");
  // Only the number is shown: the label is for a screen reader.
  const hidden = [...badge.children];
  expect(hidden.map((part) => [part.textContent, part.className])).toEqual([
    [" in dead letters", a11y.visuallyHidden],
  ]);
});

test("a count badge without a label reads the number", () => {
  const { container } = render(<CountBadge tone="accent" count={0} />);
  expect(container.firstElementChild!.textContent).toBe("0");
  expect(container.firstElementChild!.children).toHaveLength(0);
});
