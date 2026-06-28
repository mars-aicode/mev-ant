# ADR: Route sort puts total output first, with quote confidence as a tiebreaker

Status: accepted
Date: 2026-06-20
Scope: routing API behaviour — `find_routes` comparator, `Route` glossary term

## Context

`find_routes` in `src/pools/routing.rs` returns the multi-hop routes
discovered by DFS over the token graph. Each route carries `total_output`,
`total_fee_bps`, `min_pool_tvl_usd`, `quote_confidence` (Exact or
Estimated), and `hop_count`. The caller of `GET /api/routes` needs a
deterministic order: same inputs → same order, every time. The first
implementation sorted by `confidence > TVL > hops`, treating confidence
as the dominant signal because an `Estimated` route (Balancer V2/V3,
Fluid DEX in V1) is a quoter that doesn't know the on-chain math, and
a "best" route with low confidence is a worse answer than a slightly
lower-TVL `Exact` route.

The `Route` glossary entry originally listed the sort keys as
"liquidity, output amount, fee cost, and hop count" — missing confidence
entirely, listing output and fee as sort keys when neither was in the
comparator, and putting confidence as the dominant sort key with no
mention in the prose.

## Decision

Sort routes by, in order:

1. `total_output` descending — routes that pay out more win.
   `None` (no `amount_in` supplied, or a hop's quoter returned `None`)
   sorts after every `Some(_)`.
2. `total_fee_bps` ascending — among equal outputs, lower fee wins.
3. `min_pool_tvl_usd` descending — among equal output and fee, deeper
   liquidity wins.
4. `quote_confidence` descending (Exact > Estimated) — among equal
   output, fee, and TVL, the route whose quote is exact beats one
   whose quote is estimated. Treated as a tiebreaker, not a primary
   signal.
5. `hop_count` ascending — final tiebreaker.

The `Route` glossary term is rewritten to match. A new `Quote Confidence`
glossary term documents the Exact/Estimated distinction.

## Consequences

Positive:
- A high-output route wins even if confidence is lower. The caller
  asked "what's the best way to swap X for Y?" and the answer is the
  route that produces the most Y. The confidence label remains on the
  route, so the caller can filter or surface the distinction in the UI.
- Output, fee, TVL, and confidence are all in the comparator in the
  order a user would expect (best-output, then cheapest, then deepest,
  then most reliable, then shortest).
- The `None` total_output handling makes "this route couldn't be
  quoted" visible: it sorts to the bottom of the list rather than
  looking comparable to a quoted route.

Negative:
- A 1-hop `Estimated` Balancer route with a 100-USDC estimate can now
  outrank a 2-hop `Exact` UniV2+V3 route that quotes 99 USDC. The
  estimate is unreliable, so the user gets a "best" route whose
  realised output may be lower. The route label still says `Estimated`,
  so the user can see the risk. This is a deliberate trade-off: the
  caller asked for the best output, and the tiebreaker only kicks in
  for equal outputs.
- The sort is one fixed policy. Different callers (a searcher looking
  for the highest-output route, a wallet looking for the safest quote)
  may want different defaults. Tracked as Issue 0009.

## Alternatives considered

- **B-strict** (output > fee > TVL > hops, no confidence at all).
  Drops confidence from the comparator. Wrong: it makes a route with
  a wildly inaccurate estimate look identical to a confident quote
  for the same output, which is a category error.

- **Original (`confidence > TVL > hops`).** Puts confidence first.
  This was the right call when V1 only had a handful of `Exact`
  quoters, but as the registry grows to include Balancer V2/V3 and
  Fluid (both `Estimated` in V1), a TVL-rich but estimated route
  beats an exact-but-shallow route, which is the wrong answer for
  callers who care about realised output.

- **Caller-chosen sort** (`?sort=...` on `/api/routes`). The right
  long-term answer. Tracked as Issue 0009. Today's B-variant is a
  stopgap until we know which modes the product actually wants.
