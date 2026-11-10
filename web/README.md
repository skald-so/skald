# Web UI

Static SPA: TypeScript, Vite, React. Calls the API only through the client generated from [api/openapi.json](../api/openapi.json) into `src/api/`.

- `npm ci` — install dependencies from the lockfile.
- `npm run dev` — dev server; paths from the spec go to `SKALD_API_URL`, default `http://127.0.0.1:8080`.
- `npm run generate` — regenerate the API client and MSW mocks after a spec change; `npm test` fails while they are stale.
- `npm run lint`, `npm test`, `npm run build` — run by `script/test`.
- `npm run preview` — serve the build from `dist/`, forwarding the API like `dev`.
