-- Stores a space's plan entitlement. No row is needed for the default Free plan.
CREATE TYPE space_plan AS ENUM ('free', 'team');

CREATE TABLE space_entitlements (
    space_id uuid PRIMARY KEY REFERENCES spaces (id) ON DELETE CASCADE,
    plan space_plan NOT NULL
);
