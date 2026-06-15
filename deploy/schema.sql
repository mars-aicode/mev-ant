-- mev-ant database schema
-- Drop everything and recreate from scratch
DROP TABLE IF EXISTS sandwich_attackers;
DROP TABLE IF EXISTS sandwiches;
DROP TABLE IF EXISTS blocks_scanned;
DROP TABLE IF EXISTS scan_state;

-- Main sandwich bundles table
CREATE TABLE sandwiches (
    id                  BIGSERIAL PRIMARY KEY,
    block_number        BIGINT NOT NULL,
    front_tx_index      BIGINT NOT NULL,
    back_tx_index       BIGINT NOT NULL,
    victim_count        INTEGER NOT NULL,
    attacker            BYTEA NOT NULL,
    funder              BYTEA NOT NULL,
    executor            BYTEA NOT NULL,
    initiator           BYTEA NOT NULL,
    back_initiator      BYTEA NOT NULL,
    target              BYTEA NOT NULL,
    coinbase            BYTEA,
    attacked_pool       TEXT NOT NULL,
    profit_json         JSONB NOT NULL DEFAULT '[]'::jsonb,
    gas_cost_wei        NUMERIC(78,0) NOT NULL DEFAULT 0,
    coinbase_bribe      NUMERIC(78,0) NOT NULL DEFAULT 0,
    expense_wei         NUMERIC(78,0) NOT NULL DEFAULT 0,
    front_tx_hash       BYTEA,
    back_tx_hash        BYTEA,
    front_transfers     JSONB NOT NULL DEFAULT '[]'::jsonb,
    victim_transfers    JSONB NOT NULL DEFAULT '[]'::jsonb,
    back_transfers      JSONB NOT NULL DEFAULT '[]'::jsonb,
    victim_tx_hashes    JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    latest_update_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE blocks_scanned (
    block_number    BIGINT PRIMARY KEY,
    sandwich_count  INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    latest_update_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE sandwich_attackers (
    id                  BIGSERIAL PRIMARY KEY,
    attacker            BYTEA NOT NULL,
    funder              BYTEA NOT NULL,
    executor            BYTEA NOT NULL,
    initiator           BYTEA NOT NULL,
    back_initiator      BYTEA NOT NULL,
    first_block         BIGINT NOT NULL DEFAULT 0,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    latest_update_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE scan_state (
    id              INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    next_block      BIGINT NOT NULL,
    enabled         BOOLEAN NOT NULL DEFAULT true,
    chain_head      BIGINT NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    latest_update_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Seed scan_state
INSERT INTO scan_state (id, next_block) VALUES (1, 25300000);

-- Indexes
CREATE INDEX idx_sandwiches_block ON sandwiches(block_number);
CREATE INDEX idx_sandwiches_attacker ON sandwiches(attacker);
CREATE INDEX idx_sandwiches_profit ON sandwiches(gas_cost_wei);
CREATE INDEX idx_sandwiches_attacker_block ON sandwiches(attacker, block_number DESC);
CREATE UNIQUE INDEX idx_sattackers_roles ON sandwich_attackers(attacker, funder, executor, initiator, back_initiator);
CREATE INDEX idx_sattackers_first_block ON sandwich_attackers(first_block);
