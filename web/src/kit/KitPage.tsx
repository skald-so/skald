import { useLayoutEffect, useRef, useState } from "react";
import styles from "./KitPage.module.css";

export type Theme = "light" | "dark";

/** Color tokens in the order of "Tokens" in the design README. */
const colors = [
  { label: "Background", token: "--color-bg" },
  { label: "Side panel", token: "--color-side" },
  { label: "Line", token: "--color-line" },
  { label: "Text", token: "--color-text" },
  { label: "Muted text", token: "--color-muted" },
  { label: "Accent", token: "--color-accent" },
  { label: "OK", token: "--color-ok" },
  { label: "Warning", token: "--color-warning" },
  { label: "Danger", token: "--color-danger" },
];

const fonts = [
  {
    name: "Inter",
    use: "Text",
    token: "--font-text",
    size: 16,
    weights: [400, 500, 600],
    sample: "Your devices. Each has its own key.",
  },
  {
    name: "Space Grotesk",
    use: "Headings and numbers",
    token: "--font-heading",
    size: 26,
    weights: [500, 600],
    sample: "Machines 12,480",
  },
  {
    name: "JetBrains Mono",
    use: "Code, keys and IDs",
    token: "--font-mono",
    size: 15,
    weights: [400, 500],
    sample: "npx skald-so connect --code K7QM-4XWD",
  },
];

const themes: { label: string; theme?: Theme }[] = [
  { label: "System" },
  { label: "Light", theme: "light" },
  { label: "Dark", theme: "dark" },
];

/** The UI kit: every part of the web UI with fixture data. `theme` is the theme forced by the URL. */
export function KitPage({ theme }: { theme?: Theme }) {
  return (
    <main className={styles.page}>
      <header className={styles.header}>
        <div className={styles.title}>
          <h1>UI kit</h1>
          <p>Theme tokens and components of the web UI, with fixture data.</p>
        </div>
        <nav aria-label="Theme" className={styles.themes}>
          {themes.map((item) => (
            <a
              key={item.label}
              href={item.theme ? `kit.html?theme=${item.theme}` : "kit.html"}
              aria-current={item.theme === theme ? "page" : undefined}
            >
              {item.label}
            </a>
          ))}
        </nav>
      </header>

      <section aria-labelledby="kit-colors" className={styles.section}>
        <h2 id="kit-colors">Colors</h2>
        <ul className={styles.colors}>
          {colors.map((color) => (
            <li key={color.token} className={styles.card}>
              <span className={styles.name}>{color.label}</span>
              <code className={styles.token}>{color.token}</code>
              <Swatch token={color.token} scheme="light" />
              <Swatch token={color.token} scheme="dark" />
            </li>
          ))}
        </ul>
      </section>

      <section aria-labelledby="kit-fonts" className={styles.section}>
        <h2 id="kit-fonts">Fonts</h2>
        <ul className={styles.fonts}>
          {fonts.map((font) => (
            <li key={font.name} className={styles.card}>
              <span className={styles.name}>
                {font.name} <span className={styles.use}>· {font.use}</span>
              </span>
              <code className={styles.token}>{font.token}</code>
              {font.weights.map((weight) => (
                <p key={weight} className={styles.sample}>
                  <span className={styles.weight}>{weight}</span>
                  <span
                    style={{
                      fontFamily: `var(${font.token})`,
                      fontWeight: weight,
                      fontSize: font.size,
                    }}
                  >
                    {font.sample}
                  </span>
                </p>
              ))}
            </li>
          ))}
        </ul>
      </section>
    </main>
  );
}

/**
 * A token's color in one scheme, with the value the browser computes for it. The chip's own
 * color-scheme picks the value: this needs the native light-dark() of the dev server; the
 * production build resolves it on the root element only.
 */
function Swatch({ token, scheme }: { token: string; scheme: Theme }) {
  const chip = useRef<HTMLSpanElement>(null);
  const [value, setValue] = useState("");

  useLayoutEffect(() => {
    setValue(hex(getComputedStyle(chip.current!).backgroundColor));
  }, []);

  return (
    <span className={styles.swatch}>
      <span
        ref={chip}
        className={`${styles.chip} ${styles[scheme]}`}
        style={{ backgroundColor: `var(${token})` }}
      />
      <span className={styles.scheme}>
        {scheme === "light" ? "Light" : "Dark"}
      </span>
      <code>{value}</code>
    </span>
  );
}

/** `rgb(48, 70, 200)` → `#3046C8`; empty when the color is not an `rgb()` value. */
function hex(color: string): string {
  const channels = /^rgba?\((\d+), (\d+), (\d+)/.exec(color);
  if (!channels) return "";
  return `#${channels
    .slice(1)
    .map((channel) => Number(channel).toString(16).padStart(2, "0"))
    .join("")
    .toUpperCase()}`;
}
