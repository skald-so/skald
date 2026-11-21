-- The session keeps its account's username instead of the GitHub login. A session of a state
-- older than this column has no username until its next sign-in or `skald set-username`.
ALTER TABLE session DROP COLUMN github_login;
ALTER TABLE session ADD COLUMN username TEXT;
