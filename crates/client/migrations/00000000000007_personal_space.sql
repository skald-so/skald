-- The device's personal space: the one it pinned when it signed its revision 1. At most one.
ALTER TABLE space ADD COLUMN personal INTEGER NOT NULL DEFAULT 0 CHECK (personal IN (0, 1));
CREATE UNIQUE INDEX space_personal ON space (personal) WHERE personal = 1;
