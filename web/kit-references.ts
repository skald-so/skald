// Reference screenshots of the kit page and their comparison with fresh screenshots.
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";
import { themes, widths, type Theme } from "./kit-browser.ts";

export interface View {
  theme: Theme;
  width: number;
  /** The screenshot file name, e.g. `kit-light-1440.png`. */
  file: string;
}

/** Every kit page view: both themes at both check widths. */
export const views: View[] = themes.flatMap((theme) =>
  widths.map((width) => ({ theme, width, file: `kit-${theme}-${width}.png` })),
);

/** The committed references, made by `npm run kit:references`. */
export const referenceDir = join(import.meta.dirname, "kit-references");
/** Where a failed comparison saves the screenshot and its diff image; ignored by git. */
export const diffDir = join(import.meta.dirname, "kit-diff");

/**
 * Pixels whose colors are closer than this pass: the HyAB distance in OKLab, 1 for black and white.
 * The screenshots come from the same pinned Chrome, so equal pages give equal pixels; a shift of one
 * channel by 4 out of 255 is already more.
 */
const threshold = 0.005;

export interface Comparison {
  /** Both images have the same size and no pixel differs beyond anti-aliasing. */
  same: boolean;
  /** Differing pixels, anti-aliasing aside; for different sizes, over the larger area. */
  pixels: number;
  /** Size of each image, `<width>×<height>`, when they differ. */
  sizes?: { actual: string; expected: string };
  /** A PNG: the expected image faded, differing pixels red, anti-aliased ones yellow. */
  diff: Buffer;
}

/** Compares two PNG images pixel by pixel, ignoring anti-aliased pixels. */
export function compare(actual: Buffer, expected: Buffer): Comparison {
  const a = PNG.sync.read(actual);
  const e = PNG.sync.read(expected);
  const width = Math.max(a.width, e.width);
  const height = Math.max(a.height, e.height);
  const diff = new PNG({ width, height });
  const pixels = pixelmatch(
    pad(e, width, height),
    pad(a, width, height),
    diff.data,
    width,
    height,
    { threshold },
  );
  const resized = a.width !== e.width || a.height !== e.height;
  return {
    same: pixels === 0 && !resized,
    pixels,
    ...(resized && {
      sizes: {
        actual: `${a.width}×${a.height}`,
        expected: `${e.width}×${e.height}`,
      },
    }),
    diff: PNG.sync.write(diff),
  };
}

/** The RGBA pixels of `image` in a `width`×`height` area, transparent beyond the image. */
function pad(image: PNG, width: number, height: number): Uint8Array {
  if (image.width === width && image.height === height) return image.data;
  const data = new Uint8Array(width * height * 4);
  for (let y = 0; y < image.height; y++) {
    data.set(
      image.data.subarray(y * image.width * 4, (y + 1) * image.width * 4),
      y * width * 4,
    );
  }
  return data;
}

/**
 * Saves the screenshot of a failed comparison and its diff image to `dir`: `<file>` and
 * `<file without .png>.diff.png`. Returns their paths.
 */
export function saveDiff(
  dir: string,
  file: string,
  actual: Buffer,
  result: Comparison,
): { screenshot: string; diff: string } {
  mkdirSync(dir, { recursive: true });
  const screenshot = join(dir, file);
  const diff = join(dir, file.replace(/\.png$/, ".diff.png"));
  writeFileSync(screenshot, actual);
  writeFileSync(diff, result.diff);
  return { screenshot, diff };
}
