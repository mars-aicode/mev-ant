# ADR: Classifier and trace_funder design, integration test pattern

Status: accepted
Date: 2026-06-15
Scope: sandwich detector — address classification, funder resolution, regression coverage

## Context

The sandwich detector had three interlocking problems that the user surfaced
through three real mainnet blocks:

- **Block 25301029** — the `trace_funder` was attributing a sandwich to
  `0x98c23e9d…f122f4e16f5c` (Aave V3 USDC reserve proxy) instead of the
  WETH wrapper that actually fronted the capital.
- **Block 25304912** — the `trace_funder` was reporting a 0.025 ETH profit
  when the actual profit was 0.0000478 ETH. The discrepancy came from
  counting an executor→router→pool trade leg as a "funder providing
  capital".
- **Block 25300013** — the `trace_funder` was attributing a sandwich to
  `0x5ee5bf7a…ec6de8` (Aave V3 WBTC reserve proxy) instead of the user's
  own contract (`0x000000000035…357240f594e`) that fronted 727.924 WETH.

The root causes were:

1. A lending address was missing from `LENDING_ADDRESSES` (Aave V3 WBTC
   reserve), so a flashloan lender was being misidentified as a funder.
2. The flashloan guard was flagging any (to, token) round-trip as a
   flashloan, including legitimate pool trades. The legitimate
   executor→pool→executor trade in block 25300013 was being filtered out.
3. The user's contract was being misclassified as a `Pool` by the
   fund-flow classifier (back tx shows it returning WETH and paying an
   ETH bribe — disjoint tokens, which the disjoint-token branch labels
   as a swap pool). The misclassification cascaded: the contract was
   in `pool_set`, so `filter_bundles` rejected the bundle whose funder
   was that contract.
4. The 5-case `trace_funder` had no fallback for "the user contract
   itself is the funder" — only direct inbound, wrap, and borrow were
   handled. When the user contract didn't match any of those, the
   detector fell back to the executor (self-funded), which was wrong.

The integration test infrastructure also needed to be in place: the
detector's regressions were being discovered by the user after the fact
and fixed ad-hoc. We needed a way to lock in the fixes as regression
tests that run automatically.

## Decision

### Five-case `trace_funder`

The `trace_funder` resolves the at-risk capital source in this strict
order; the first hit that survives the round-trip / sufficiency check
wins:

1. **Direct inbound** — a single counterparty sent the pool-out token to
   the executor in an amount that covers the executor's pool-out. This
   is the common case (EOA → executor).
2. **Wrap** — the executor's outbound is WETH, but the executor's
   pre-balance was ETH. Trace inbound ETH and unwrap to find the
   wrap-provider.
3. **Borrow** — the executor deposits the inbound token as collateral
   on a lending platform, borrows a different token, and trades. The
   funder is the original depositor (the EOA).
4. **Real capital** — fallback for "the real funder isn't a direct
   sender, wrap, or borrow". Pick the largest non-pool, non-lending,
   non-token-contract inbound whose token matches the executor's
   outbound and whose amount covers the executor's outbound in the
   same token. This is what catches the block 25300013 user-contract
   pattern: the user contract sends 727.924 WETH, the executor sends
   727.924 WETH to Aave (collateral deposit, not a pool trade), and
   the original trade tokens (USDC, cbBTC) are flashloaned.
5. **tx.target as funder** — the front tx's `to` is the EOA's call
   destination, typically the user's own contract that orchestrates
   the sandwich. When the target isn't a lending platform, isn't the
   executor, isn't zero, and has `→executor` transfers in the front
   tx, the target is the funder. This recovers the user as the actual
   funder when their contract has been misclassified as a Pool by the
   classifier and is therefore unreachable via cases 1-4.

If all five cases miss, the executor is the funder (self-funded).

### Token contract exclusion in case 4

A token contract (WETH contract, USDC contract, etc.) is *not* a
funder — it's protocol plumbing. A WETH unwrap emits a
`Transfer(WETH_contract → executor, ETH, amount)` whose `from` is the
WETH contract; the contract is the sender, not the capital source.

The exclusion check: `!supported_tokens.contains(&t.from)`. The
`sender` of the transfer must not be a known token contract. We use
`supported_tokens` (the set of contract addresses for WETH, USDC,
USDT, DAI, WBTC, plus the ETH sentinel) as the proxy for "known token
contract", since in practice the set of token contracts the detector
sees is exactly the supported set.

### Sufficiency check in case 4

An EOA that sent 99 wei of dust to the executor is not a funder.
The candidate's inbound must cover the executor's outbound in the same
token. This is the same sufficiency check case 1 uses, applied to the
fallback.

### Pool-aware flashloan guard

A normal trade has executor→pool and pool→executor transfers of the
same token. The old guard flagged any (to, token) match as a
flashloan, which dropped legitimate sandwiches like block 25300013
where the executor's cbBTC↔Balancer round-trip is the trade itself.

The fix: filter the `senders` and `repays_same` sets to exclude pool
counterparties. A round-trip with a pool is the trade; a round-trip
with a non-pool actor is a flashloan.

### Removal of `!pool_set.contains(&funder)` in `filter_bundles`

With case 5 in place, the funder can legitimately be a contract that
the classifier mislabeled as a Pool. The `filter_bundles` check
rejected those bundles silently. The check is redundant: `trace_funder`
already has case 5 specifically to recover user contracts from
misclassification, and a funder that's actually a real pool would
fail the case-5 transfer check (no inbound from a real pool to the
executor in a normal sandwich).

### Per-token amount equality for Router classification

The fund-flow classifier labels an address as a Router when it sees the
same token set flowing in and out (`sent_tokens == recv_tokens`). This
replaced an earlier "any-overlap" heuristic that was too loose.

Block 25302239 exposed that strict token-set equality is still too
loose: the real sandwich executor handles WETH, ETH, and one traded
token in both directions, but with non-zero net amounts (it keeps the
trade profit). The token sets matched, so the executor was classified
as a Router, moved into `pool_or_router`, and skipped by executor
discovery.

The refinement: after token-set equality, require **per-token amount
equality** too (`total_sent_per_token == total_received_per_token`). A
pure passthrough router has matching amounts for every token it
forwards; an executor with wrap/route bookkeeping and a profit does
not. This keeps real executors in `unknown` while still catching true
same-token routers.

### Classifier skip of `Address::ZERO`

A pre-fix bug: the zero address is the ERC20 mint/burn sentinel. Its
apparent same-token in/out pattern is an artifact, not a passthrough.
The fund-flow classifier was labeling it as a Router (same-token
passthrough), which leaked it into `pool_or_router` and broke the
attacked_pool triple intersection (the postprocess would return
`0x0000…` non-deterministically when more than one pool matched). The
classifier now skips `Address::ZERO` in fund-flow analysis.

### Integration test pattern

Tests live in `src/integration_tests.rs`, declared as
`#[cfg(test)] mod integration_tests;` from `src/main.rs`. Each test
is wrapped in an `integration_test!` macro that:

1. Reads `MEV_ANT_RPC_URL` (default `http://192.168.2.180:8547`).
2. Probes the RPC URL with a TCP connect (1s timeout).
3. Skips the test (`return`) if unreachable — no `#[ignore]`, no
   compile-time guard. Tests are zero-cost when Reth is down.
4. Calls `detect_sandwiches` directly, bypassing the scanner and DB.

Trade-off: tests are network-dependent and slower (~0.5s each) but
zero committed bytes (no JSON fixtures to drift) and always current
against live mainnet state. Each test documents a real block that
the user reported and the specific assertion that locks in the fix.

Adding a new regression:

1. Pick the block and the user-reported bug.
2. Write a test that calls `detect(&client, block_number)`.
3. Assert the bundle exists, the funder is the expected address, the
   executor / victim / pool match the user's report.
4. Add `#[tokio::test]`-free macro wrap so it auto-skips when Reth
   is down.

The four current regressions each lock in one bug class:

- **25301029** — lending address missing from `LENDING_ADDRESSES`.
- **25304912** — dust EOA incorrectly picked as funder (case 4
  sufficiency + token-contract exclusion).
- **25300013** — user contract misclassified as Pool; tx.target
  fallback in case 5.
- **25302239** — real executor misclassified as Router because the
  classifier only checked token-set equality; per-token amount equality
  refinement keeps the executor discoverable.

## Consequences

Positive:

- Each case in `trace_funder` has a clear, named responsibility; the
  order is the priority order.
- New lending platforms need only an entry in `LENDING_ADDRESSES` —
  no code change.
- New user-contract patterns are recovered by case 5 without
  classifier changes.
- Integration tests lock in the user's bug reports as executable
  regression coverage.

Negative:

- Case 5 trusts `tx.to` as the funder. A future pattern where a
  router is `tx.to` could over-fire. The check requires
  `target != lending && target != executor && has inbound to executor`
  to mitigate.
- Token-contract exclusion uses `supported_tokens` as the proxy for
  "known token contract". A new stablecoin not in `DEFAULT_TOKENS`
  would still be picked as a funder by case 4. Acceptable trade-off
  for the supported-token profit universe.
- Live-Reth integration tests are slower than JSON fixtures and
  depend on the user's local Reth being current. Acceptable: the
  detector's correctness is tied to live mainnet, and JSON fixtures
  would drift.

## Alternatives considered

- **Skip `tx.to` in classifier fund-flow analysis.** Would fix
  block 25300013 (user contract wouldn't be misclassified as Pool)
  but broke block 25304912 (the router is `tx.to` for some aggregator
  swaps; skipping it left the router in `unknown` and case 4 picked
  it as a funder). The case-5 fallback in `trace_funder` is a more
  targeted fix: it recovers the user contract from misclassification
  *for the funder resolution* without changing the classifier.

- **JSON fixtures for integration tests.** Faster, deterministic, no
  network dependency. Rejected: the detector's correctness is tied
  to live mainnet, and the fixtures would drift as new DEXes and
  patterns appear. Live-Reth tests at ~0.5s each is acceptable for
  the regression coverage gained.

- **Stricter round-trip check in `trace_funder`.** Instead of a
  5-case resolver, add a per-token round-trip filter to case 1.
  Rejected: doesn't address the case-4 fallthrough and the case-5
  user-contract recovery; the case-based structure is clearer to
  reason about than a more elaborate case 1.

- **Keep `!pool_set.contains(&funder)` in `filter_bundles`.** Would
  require the classifier to be perfect. Rejected: the classifier is
  necessarily approximate (it operates on a single tx's transfer
  graph), so the postprocess can't rely on perfect upstream
  classification.
