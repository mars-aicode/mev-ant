# ADR: Live-Reth integration tests fail loudly

Status: accepted
Date: 2026-06-16
Scope: test strategy — unit tests vs. integration tests, CI requirements

## Context

The project has two test layers:

- **Unit tests** exercise individual seams (classifier, funder resolver,
  detector post-processing) with in-memory fixtures. They run offline and
  fast.
- **Integration tests** in `src/integration_tests.rs` run the full
  `fetch_block → detect_sandwiches` pipeline against a live Reth node on
  real mainnet blocks.

Originally the integration tests auto-skipped when Reth was unreachable.
This made local `cargo test` convenient but created a silent-dependency
risk: CI could pass without ever exercising the regression suite if the
Reth node was missing or misconfigured.

## Decision

Integration tests **fail loudly** when Reth is unreachable. They require
`MEV_ANT_RPC_URL` to be set, or the default URL
(`http://192.168.2.180:8547`) to be reachable. If neither is available,
the test panics immediately with a clear message instead of skipping.

Unit tests remain offline and must not depend on Reth.

### Why live data is non-negotiable

The detector's correctness is tied to real mainnet trace and log formats.
JSON fixtures would:

- Drift as new DEX patterns and event signatures appear.
- Commit large binary data into the repo.
- Give false confidence when a heuristic happens to pass on a stale fixture
  but fails on current blocks.

The project explicitly prioritizes a small git repo and real-data
validation over deterministic fixtures.

### Why fail loudly

A skipped test is an invisible test. CI must guarantee that the Reth node
is reachable; failing loudly makes a missing node an explicit failure
rather than a green build with zero regression coverage.

### Offline coverage is still required

Before integration tests can fail, the offline unit-test layer must be
strong enough to catch logic regressions without the network. After this
ADR, the offline suite includes:

- Classifier tests (event, blacklist, lending, fund-flow router/pool).
- Funder resolver tests (all five cases and round-trip guards).
- Detector post-process tests (dedup, blacklist filter, zero-victim drop,
  overlap resolution).

## Consequences

Positive:

- CI cannot silently pass with broken/missing Reth access.
- Developers have a clear contract: `cargo test` runs offline logic;
  `cargo test integration` requires a node.
- The repo stays small — no fixtures committed.

Negative:

- Local `cargo test integration` fails if the developer's Reth node is
  down. The error message points to `MEV_ANT_RPC_URL`.
- CI must provision and keep a Reth node reachable.

## Alternatives considered

- **Keep auto-skip.** Rejected: silent skips defeat the purpose of
  regression tests in CI.
- **Add a `MEV_ANT_RPC_URL=skip` opt-out.** Rejected at this time: the
  user chose strict failure as the default. A skip env var can be
  reconsidered later if local ergonomics become painful.
- **Record real traces into `/tmp` fixtures.** Rejected: the user wants
  real data *and* a small repo; generated fixtures in `/tmp` are
  ephemeral and don't help CI determinism without a separate recording
  step.
- **Split integration tests into a separate crate with `#[ignore]`.**
  Rejected: running them would still require an explicit flag, and the
  panic-on-missing-RPC behavior inside the existing macro is simpler.
