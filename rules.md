# Rules

1. **Documentation, commit messages, PR descriptions and PR comments are as concise and clean as possible.**
   - Facts and decisions only: no intros, filler, repetition or reasoning.
   - Short sentences and lists, one idea per item.
   - Each fact lives in one place; everywhere else links to it.
   - Outdated content is deleted, not marked.
   - A PR description or comment says what changed, what was decided and what is open, in a few bullets: no restated task, plan or diff, no check logs beyond a red check's failures.
2. **Work is done only when `script/test` passes in full.**
   - Tests cover required behavior in the same iteration and catch every breakage of it.
   - Required behavior is what the Definition of done or a spec asks for.
   - For a PR outside the plan, its description is its Definition of done.
   - Tests are never deleted, skipped or weakened to get a green run.
3. **PR titles follow Conventional Commits:** `<type>(<scope>): <summary>`, e.g. `feat(server): deliver envelopes to offline devices`. A squash merge turns the title into the commit in `dev`; release-plz derives the version and the CHANGELOG from it.
4. **An API change starts with the OpenAPI spec** ([`api/openapi.json`](api/openapi.json)). It is the contract: the server implements it, the web uses the client and mocks generated from it.
5. **Critical stories get acceptance tests before code.**
   - A story is critical if a defect in it can leak or lose data or bypass access: cryptography and keys, delivery and receipts, queue claims, sign-in, devices and access rights, data deletion.
   - Its body has the line `- **Acceptance tests:** first`. The planner sets it; the plan's approver confirms it.
   - The acceptance tests check the Definition of done through public interfaces. The author of the code changes one only to fix a defect in the test, with the reason in the PR.
6. **The owner checks some stories by hand.**
   - Such a story has the line `- **Manual check:** <what to check>`; the owner sets it.
   - After the story is done (rule 7) and `script/test` passes on `dev`, the owner checks it and deletes the line once the check passes.
   - A story starts when its "Depends on" stories are done and none of them has the line.
   - A check on a story nothing depends on holds its release.
7. **A review blocks a PR only on defects.**
   - Issues: the Definition of done is not met; a breach of rules 2–5; a red CI check or a flaky test; wrong behavior or a wrong fact; a security or privacy risk.
   - Everything else is a nit, e.g. duplicated code, a repeated fact, the PR description's form, naming, an untested edge case of behavior nothing requires.
   - A PR whose review found only nits is merged once they are fixed, without another review.
   - A re-review checks only the fix's diff: that it resolves the previous issues and adds none.
   - In a prototype, a mishandled edge case that does not change the iteration's answer is a nit: the author adds it to the report as an open question.
   - After a story's last iteration, a PR of its own removes duplication across the story's iterations; the story is done once it is merged, or after its last iteration if there is none to remove.
   - This rule overrides a review tool's own split of issues and nits, its re-review scope and when it counts a story done.
