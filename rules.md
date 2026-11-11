# Rules

1. **Documentation is as concise and clean as possible.**
   - Facts and decisions only: no intros, filler, repetition or reasoning.
   - Short sentences and lists, one idea per item.
   - Each fact lives in one place; everywhere else links to it.
   - Outdated content is deleted, not marked.
2. **Work is done only when `script/test` passes in full.**
   - New behavior is covered by tests in the same iteration.
   - Tests are never deleted, skipped or weakened to get a green run.
   - Tests catch broken behavior: a breakage of the changed code that no test notices is a review issue.
3. **PR titles follow Conventional Commits:** `<type>(<scope>): <summary>`, e.g. `feat(server): deliver envelopes to offline devices`. A squash merge turns the title into the commit in `dev`; release-plz derives the version and the CHANGELOG from it.
4. **An API change starts with the OpenAPI spec** ([`api/openapi.json`](api/openapi.json)). It is the contract: the server implements it, the web uses the client and mocks generated from it.
5. **Critical stories get acceptance tests before code.**
   - A story is critical if a defect in it can leak or lose data or bypass access: cryptography and keys, delivery and receipts, queue claims, sign-in, devices and access rights, data deletion.
   - Its body has the line `- **Acceptance tests:** first`. The planner sets it; the plan's approver confirms it.
   - The acceptance tests check the Definition of done through public interfaces. The author of the code changes one only to fix a defect in the test, with the reason in the PR.
6. **The owner checks some stories by hand.**
   - Such a story has the line `- **Manual check:** <what to check>`; the owner sets it.
   - After the story's gate, the owner checks it and deletes the line once the check passes.
   - A story starts when its "Depends on" stories are done and none of them has the line.
   - A check on a story nothing depends on holds its release.
