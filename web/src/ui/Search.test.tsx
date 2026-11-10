import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, expect, test, vi } from "vitest";
import { Search } from "./Search.tsx";

afterEach(cleanup);

/** A screen that keeps the query and reports each change to `onChange`. */
function Screen({
  initial = "",
  onChange,
}: {
  initial?: string;
  onChange: (query: string) => void;
}) {
  const [query, setQuery] = useState(initial);
  return (
    <Search
      label="Search accounts"
      placeholder="Search by name, login or email"
      value={query}
      onChange={(next) => {
        onChange(next);
        setQuery(next);
      }}
    />
  );
}

const field = () =>
  screen.getByRole<HTMLInputElement>("searchbox", { name: "Search accounts" });
const clear = () => screen.queryByRole("button", { name: "Clear search" });

test("is a search field with its label and hint", () => {
  render(<Screen onChange={() => {}} />);

  expect(field().placeholder).toBe("Search by name, login or email");
  expect(field().value).toBe("");
});

test("reports the query as the user types", async () => {
  const onChange = vi.fn();
  render(<Screen onChange={onChange} />);

  await userEvent.setup().type(field(), "Lee");
  expect(onChange.mock.calls).toEqual([["L"], ["Le"], ["Lee"]]);
  expect(field().value).toBe("Lee");

  await userEvent.setup().keyboard("{Backspace}");
  expect(onChange.mock.calls.at(-1)).toEqual(["Le"]);
});

test("shows the clear button only while there is a query", async () => {
  render(<Screen onChange={() => {}} />);
  expect(clear()).toBeNull();

  await userEvent.setup().type(field(), "a");
  expect(clear()).not.toBeNull();
  await userEvent.setup().keyboard("{Backspace}");
  expect(clear()).toBeNull();
});

test("the clear button reports an empty query and returns the focus to the field", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  render(<Screen initial="ana" onChange={onChange} />);

  await user.click(clear()!);
  expect(onChange.mock.calls).toEqual([[""]]);
  expect(field().value).toBe("");
  expect(clear()).toBeNull();
  expect(document.activeElement).toBe(field());
});

test("the clear button works from the keyboard", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  render(<Screen initial="ana" onChange={onChange} />);

  await user.tab();
  expect(document.activeElement).toBe(field());
  await user.tab();
  expect(document.activeElement).toBe(clear());
  await user.keyboard("{Enter}");
  expect(onChange.mock.calls).toEqual([[""]]);
  expect(document.activeElement).toBe(field());
});

test("Escape clears the query and keeps the focus in the field", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  const onOuterKey = vi.fn();
  render(
    <div onKeyDown={(event) => onOuterKey(event.key)}>
      <Screen onChange={onChange} />
    </div>,
  );

  await user.type(field(), "ana");
  onChange.mockClear();
  await user.keyboard("{Escape}");
  expect(onChange.mock.calls).toEqual([[""]]);
  expect(field().value).toBe("");
  expect(document.activeElement).toBe(field());
  // Escape that cleared the query does not reach the page around the field.
  expect(onOuterKey).not.toHaveBeenCalledWith("Escape");

  // In an empty field Escape reports nothing and goes on, as to close a dialog.
  await user.keyboard("{Escape}");
  expect(onChange.mock.calls).toEqual([[""]]);
  expect(onOuterKey).toHaveBeenCalledWith("Escape");
});

test("other keys type, not clear", () => {
  const onChange = vi.fn();
  render(<Screen initial="ana" onChange={onChange} />);

  fireEvent.keyDown(field(), { key: "Enter" });
  expect(onChange).not.toHaveBeenCalled();
  expect(field().value).toBe("ana");
});

test("takes the width the screen sets", () => {
  render(
    <Search label="Search messages" value="" onChange={() => {}} width={240} />,
  );

  const frame = screen.getByRole("searchbox").parentElement!;
  expect(frame.style.getPropertyValue("--search-width")).toBe("240px");
});
