# Rules

1. **Documentation is as concise and clean as possible.**
   - Facts and decisions only: no intros, filler, repetition or reasoning.
   - Short sentences and lists, one idea per item.
   - Each fact lives in one place; everywhere else links to it.
   - Outdated content is deleted, not marked.
2. **Work is done only when `script/test` passes in full.**
   - New behavior is covered by tests in the same iteration.
   - Tests are never deleted, skipped or weakened to get a green run.
3. **PR titles follow Conventional Commits:** `<type>(<scope>): <summary>`, e.g. `feat(server): deliver envelopes to offline devices`. A squash merge turns the title into the commit in `dev`; release-plz derives the version and the CHANGELOG from it.
4. **An API change starts with the OpenAPI spec** ([`api/openapi.json`](api/openapi.json)). It is the contract: the server implements it, the web uses the client and mocks generated from it.
