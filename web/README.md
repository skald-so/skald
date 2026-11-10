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
- `npm run kit:screenshots [-- <dir>]` saves the kit page in both themes at 1440 and 360 px, default `kit-screenshots/`; reviews compare them with the approved design screenshots, `design/png/` in `skald-cloud`.
- The kit tests and screenshots run Google Chrome headless; `SKALD_CHROME` points to another Chrome or Chromium binary.
