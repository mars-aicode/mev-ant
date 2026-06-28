# Issue 0009 — Caller-chosen route sort mode

## Goal
Let the caller of `GET /api/routes` pick the sort key for the returned
routes. The current implementation uses a fixed sort policy
(`output > fee > TVL > confidence > hops`, per `find_routes` in
`src/pools/routing.rs`); the right answer is mode-dependent on the
caller's use case.

## Acceptance Criteria
- [x] `GET /api/routes` accepts a `sort` query parameter with values
      `output | fee | tvl | confidence | hops` (default `output` to
      preserve current behaviour).
- [x] The pathfinder returns an unsorted `Vec<Route>`,
      and `sort_routes` applies the chosen comparator.
- [x] The default mode (`output`) matches the B-variant behaviour shipped
      in Issue 0008's follow-up: `output (desc) > fee (asc) > TVL (desc)
      > confidence (asc) > hops (asc)`.
- [x] Each sort mode is unit-tested in `src/pools/routing.rs::tests`:
      pick a graph with 2–3 routes whose sort keys differ, assert the
      chosen mode produces the documented order.
- [x] `RouteListResponse` includes a reflected `sort` field so callers
      can confirm the mode used.
- [ ] Update the `Route` glossary entry to list the available modes
      (or split "Route" into "Route" and "Route Sort Mode").
- [ ] Update `docs/failure-modes.md` with a "bad sort key" failure mode
      (unknown `?sort=` value → 400 with a clear error).

## Dependencies
- Issue 0008 (the B-variant sort shipped in the follow-up).

## Notes / Risks
- The current `find_routes` returns a sorted `Vec<Route>`. Splitting
  build from sort is a one-shot refactor; the unit tests already pin
  the order in the default mode, so the refactor should be observable
  only via a new test that exercises a non-default mode.
- This is a product question, not a model question. If the team decides
  the fixed B-variant sort is the only one we ever need, drop the
  feature and keep the B-variant sort as-is.
- The B-variant sort is documented in ADR 0004; this issue implements
  one of the alternatives that ADR considered.
