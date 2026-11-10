# Web UI

Static SPA: TypeScript, Vite, React. Calls the API only through the client generated from [api/openapi.json](../api/openapi.json) into `src/api/`.

- `npm ci` — install dependencies from the lockfile.
- `npm run dev` — dev server; paths from the spec go to `SKALD_API_URL`, default `http://127.0.0.1:8080`.
- `npm run generate` — regenerate the API client and MSW mocks after a spec change; `npm test` fails while they are stale.
- `npm run lint`, `npm test`, `npm run build` — run by `script/test`.
- `npm run preview` — serve the build from `dist/`, forwarding the API like `dev`.

## UI kit

- Components live in `src/ui/`.
- The kit page shows every part of the UI with fixture data: `/kit.html` on the dev server, e.g. `http://localhost:5173/kit.html`. It is not in the production build.
- The theme follows the system setting; `?theme=light` or `?theme=dark` forces one.
- `?screen=cabinet` shows the cabinet shell around the Machines screen; `?screen=cabinet-menu` opens its side menu at phone width.
- `npm run kit:screenshots [-- <dir>]` saves every view, default `kit-screenshots/`: the kit page and the cabinet in both themes at 1440 and 360 px, the open side menu in both themes at 360 px. Reviews compare them with the approved design screenshots, `design/png/` in `skald-cloud`.
- The kit tests run Google Chrome headless; `SKALD_CHROME` points to another Chrome or Chromium binary.

## Reference screenshots

- `kit-references/` holds every view of `npm run kit:screenshots`. `npm test` takes the same screenshots and compares them pixel by pixel; any difference beyond anti-aliasing fails.
- A failed view saves its screenshot and a diff image, differing pixels in red, to `kit-diff/`; CI uploads them as the `kit-diff` artifact.
- Screenshots come from the Chromium of the Playwright Docker image pinned in `kit-browser.ts`, so macOS and CI render the same pixels; Docker must be running. The image runs as linux/amd64, emulated on Apple silicon.
- After an intended visual change, `npm run kit:references` updates the references; commit them in the same PR. A reviewer compares the changed references with the design in `skald-cloud`.
- A playwright-core update needs the image of the same version in `kit-browser.ts` and new references.
