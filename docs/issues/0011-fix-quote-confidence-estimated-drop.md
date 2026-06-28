# Issue 0011 — Fix Quote Confidence: Non-exact routes are silently dropped

## Problem Statement

When querying `/api/routes?from=0x...&to=0x...&amount=...`, routes that pass
through a pool without an exact output quoter (e.g., Fluid DEX, Balancer V2/V3)
are silently dropped from the response. Users see fewer routes than actually
exist in the pool graph, and cannot distinguish between "no route exists" and
"a route exists but its output cannot be quoted exactly."

The `CONTEXT.md` glossary defines `QuoteConfidence::Estimated` for exactly this
case, but the code doesn't surface those routes at all.

## Solution

When `build_route` encounters a hop whose `quote_exact_output` returns `None`,
instead of discarding the entire route with `return None`, continue building
the route with `total_output = None` and `quote_confidence = Estimated`. This
matches the behaviour when `amount_in` is not supplied.

Routes with `total_output = None` already sort after all quoted routes (per
the existing sort comparator), so they don't displace routes with known output.

## Acceptance Criteria
- [x] Fluid pool routes survive when `amount_in = Some`.  (test: `fluid_pool_survives_with_amount_in`)
- [x] Unquotable hops set `has_output = false` and mark confidence as `Estimated`.
- [x] Confidence tiebreaker direction fixed: `a.cmp(&b)` (ascending, Exact first)
  instead of `b.cmp(&a)` (descending, which put Estimated first due to Ord layout).
- [x] All existing routing tests pass (13 + 5 new sort tests = 18 total).
- [x] `CONTEXT.md` `Quote Confidence` entry updated to document the bug and its
  fix.

## User Stories

1. As a routing API consumer, I want to see routes through Fluid DEX pools
   when `amount` is supplied, so that I can discover all available swap paths
   even when exact output quoting is unavailable.
2. As a dashboard user, I want `Estimated` routes to appear alongside `Exact`
   ones (sorted after them), so that I understand the full connectivity of
   the pool graph.
3. As an API consumer supplying `amount_in`, I want `total_output = None`
   routes to be clearly distinguished from ones with computed output, so
   that I can decide whether to accept the estimation risk.

## Implementation Decisions

- **Seam**: `build_route` in the routing module — one function, one control-flow
  change.
- **Change**: replace `return None` (line 124) with `all_exact = false` and
  continue building the route without advancing the `amount` accumulator.
- **Confidence label**: route gets `QuoteConfidence::Estimated`; `total_output`
  remains `None` (the output cannot be computed when a hop's quoter returns
  `None`).
- **Sorting**: no change needed — `total_output = None` already sorts after
  `Some(_)` via `Option<U256>` ordering.
- **No schema or API contract changes**: the `Route` response shape is unchanged.
- **No DB migration**: routing is purely read-side.

## Testing Decisions

- **What makes a good test**: construct a `TokenGraph` containing a pool whose
  kind has no quoter (Fluid or a test-only pool kind), call `find_routes` with
  `amount_in = Some(...)`, assert the route is returned with
  `quote_confidence = Estimated` and `total_output = None`.
- **Prior art**: `fluid_pool_is_estimated` test already covers the
  `amount_in = None` case. The new test complements it by covering
  `amount_in = Some`.
- **No integration test needed**: the routing module is deterministic and
  requires no external infrastructure.

## Out of Scope

- Adding an approximate quoter for Balancer or Fluid pools (separate issue).
- Changing the route sort order (ADR 0004 governs sort; non-exact routes
  already sort last).
- Surfacing `QuoteConfidence` in the sandwich detection pipeline (unrelated).

## Further Notes

- The `CONTEXT.md` glossary already documents the *intended* behaviour — the
  code was simply not matching the spec. After fixing, the glossary entry for
  `Quote Confidence` can have its `CURRENT BUG` annotation removed.
