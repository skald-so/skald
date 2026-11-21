-- The limit on replies without a human (`docs/protocol/README.md`, "Content"): the `hops` of
-- each kept message and own message waiting for the server, none for one an older client kept;
-- and, for each agent in each space, the highest `hops` of the messages handed over to it since
-- the device last sent a message as it, no row when none was; a member service agent's under
-- [`SERVICE_AGENT`].
ALTER TABLE kept ADD COLUMN hops INTEGER CHECK (hops >= 0);
ALTER TABLE own_message ADD COLUMN hops INTEGER CHECK (hops >= 0);
CREATE TABLE agent_hops (
    space TEXT NOT NULL,
    agent TEXT NOT NULL,
    highest INTEGER NOT NULL CHECK (highest >= 0),
    PRIMARY KEY (space, agent)
) STRICT
