-- Each web session records the sign-in method it signed in with, and ends with it.

-- The web sessions from before have no method that could be told: they end. Device sessions stay.
DELETE FROM sessions WHERE kind = 'web';
ALTER TABLE sessions
    ADD COLUMN method_provider text,
    ADD COLUMN method_subject text,
    ADD CHECK ((method_provider IS NULL) = (method_subject IS NULL)),
    ADD CHECK ((kind = 'web') = (method_provider IS NOT NULL)),
    ADD FOREIGN KEY (method_provider, method_subject)
        REFERENCES sign_in_methods (provider, subject) ON DELETE CASCADE ON UPDATE CASCADE;
CREATE INDEX sessions_method ON sessions (method_provider, method_subject);
