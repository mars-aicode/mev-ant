# Issue 0010 — Gas cost double-counts priority fee

## Goal
Fix a long-standing calculation bug in the sandwich detector's `Cost`
aggregation. The total `Cost` is currently the sum of two
overlapping expressions:

- `gas_cost_wei` (line 431): `gas_used * (effective_gas_price + effective_priority_fee)`
- `coinbase_bribe` (line 435): `gas_used * effective_priority_fee`

Under EIP-1559, `effective_gas_price = base_fee + priority_fee`, so
`effective_gas_price + effective_priority_fee = base_fee + 2 * priority_fee`.
The priority fee is counted twice. The `Net` figure (defined in
`CONTEXT.md` as `Profit - Cost`) is therefore systematically too low by
`gas_used * priority_fee` per attacker transaction.

## Acceptance Criteria
- [ ] Replace `gas_cost_wei` with `gas_used * effective_gas_price`
      (no `+ effective_priority_fee`).
- [ ] Keep `coinbase_bribe` as the priority-fee aggregate.
- [ ] Add a unit test that pins the corrected formula against a known
      block: pick a block whose sandwich has a known
      `effective_gas_price` and `effective_priority_fee`; assert the
      recomputed `gas_cost_wei` matches the test's hand calculation.
- [ ] Add a unit test that asserts `gas_cost_wei + coinbase_bribe` is
      strictly less than the old (buggy) sum for the same block — a
      regression catch.
- [ ] Update the `Cost` glossary entry to clarify that `gas_cost_wei` is
      `gas_used * base_fee` (the non-priority component) and
      `coinbase_bribe` is `gas_used * priority_fee + direct ETH bribes`.
      (The two still sum to the attacker's total spend; the bug was
      that `gas_cost_wei` was double-counting the priority.)

## Dependencies
- None. Pre-existing bug carried over from `building.rs`; flagged by
  the post-Issue-0008 code review.

## Notes / Risks
- The fix changes observable numbers in `/api/sandwiches` and
  `/api/attackers` for every existing sandwich. Any historical
  "top sandwiches by profit" dashboard will re-rank. That's the
  intended behaviour — the old numbers were wrong.
- The live integration tests (`block_25304912_dust_funder_self_funded`
  etc.) assert specific profit/net values; check whether any of them
  depended on the old formula and update the expected values.
- `Net` was defined in `CONTEXT.md` as `Profit - Cost`. After the fix,
  `Net` will be larger for every bundle; update any test that asserts
  a specific `Net`.
