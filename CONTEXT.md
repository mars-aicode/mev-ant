# MEV Ant

Historical sandwich MEV scanner for Ethereum mainnet.

## Language

**Sandwich Bundle**:
Three or more consecutive transactions in a single block sharing the same attacker.
Frontrun trade → ≥1 victim trade → backrun trade. All three must be separate transactions.
Single-tx atomic sandwiches are NOT sandwiches.
_Avoid_: sandwich attack, MEV bundle

**Frontrun / Backrun**:
The two attacker transactions bracketing a victim. Frontrun performs the opening trade
(price moves against victim), backrun does the closing reversal (price returns + profit
extracted). Both share the same attacker.
_Avoid_: pre-trade/post-trade, open/close

**Attacker**:
The funder contract that provides capital for the frontrun and receives profit from
the backrun. Traced from the front TX's transfer graph: who sent assets to the executor?
May differ from initiator. May equal executor when self-funded.
_Avoid_: searcher, bot, bundler, EOA

**Initiator**:
The EOA (tx.from) that relays a transaction. Front and back may have different
initiators when paired by same target. Recorded as `initiator` and `back_initiator`.
_Avoid_: worker, sender, relay, EOA

**Funder**:
The address that provides assets into the executor for the frontrun trade.
Traced from non-pool transfers. Only Unknown addresses (not Token, Pool, or Infra)
can be funders. When the executor self-funds, funder = executor.
Attacker = funder. A round-trip provider (sends token X and receives token X back
in same tx) is NOT a funder — it's a flashloan lender.
_Avoid_: capital provider

**Flashloan**:
Borrowed capital that must be repaid in the same tx. Distinguished from funder
by same-token round-trip (provider sends and receives same token from executor).
Self-funded bundles with flashloan patterns are excluded from sandwich detection.
_Avoid_: borrowed capital

**Executor**:
The contract that directly touches the DEX pool (transfers tokens to/from pool
addresses). Executes the actual swap logic. May temporarily store assets between
frontrun and backrun when funder ≠ executor. Discovered via pool-involved transfers.
_Avoid_: bot contract, swap contract

**Attacked Pool**:
The DEX pool whose price is manipulated to extract value from the victim.
Must be involved in frontrun, victim, AND backrun phases.
_Avoid_: victim pool, target pool, manipulated pool

**Auxiliary Pool**:
A DEX pool that an attacker touches only inside the frontrun or backrun
transaction, in addition to the Attacked Pool, for routing or
entry/exit in a multi-hop trade. Recorded on the sandwich bundle as
`auxiliary_pools`. Distinct from a Liquid Pool: an Auxiliary Pool is
attacker-only and not necessarily large, whereas a Liquid Pool is any
top-1,000-by-TVL DEX pool regardless of whether a sandwich touched it.
_Avoid_: liquidity pool (collides with the registry term), routing pool

**Liquid Pool**:
A DEX pool tracked by the liquidity-registry feature because it ranks in the
global top 1,000 by TVL. Liquid Pools feed the routing API and may be used by
any MEV strategy (arbitrage, sandwich routing, etc.). Distinct from the
sandwich-specific "Auxiliary Pool" and the broader "Tracked Pool".
_See also_: Auxiliary Pool, Tracked Pool.
_Avoid_: high-liquidity pool, deep pool

**Tracked Pool**:
A DEX pool registered in the registry's `pools` table — discovered by the
classifier (via Swap/Mint/Burn events) or seeded by the bootstrap file.
A Tracked Pool may or may not be a Liquid Pool: any pool the registry has
seen is Tracked, but only the top 1,000 by TVL are Liquid. The
`GET /api/pools/:pool` endpoint serves any Tracked Pool; the
`/api/liquid-pools/:pool` endpoint serves only Liquid Pools. Sandwich
bundles link to `/api/pools/:pool` because a sandwich's `attacked_pool`
can reference any pool the classifier has seen, including long-tail
pools that never reach the top 1,000.
_See also_: Liquid Pool, Pool, Pool Snapshot.
_Avoid_: registered pool, known pool

**Pool**:
The logical DEX pool where a swap executes. For UniV2/V3 the pool is identified
by its contract address. For Balancer/UniV4 vaults the pool is identified by its
bytes32 Pool ID, but executed via the shared Vault/PoolManager contract address.
Discovered via classifier (receipt-level Swap event topic0 matching).
_See also_: Pool Snapshot, Liquid Pool.
_Avoid_: market, pair, venue

**Pool Address**:
The contract address used to interact with a pool. For UniV2/V3 this is the pool
contract itself. For Balancer V2/V3 this is the Vault. For UniV4 this is the
PoolManager. Always present; distinct from Pool ID when a vault singleton hosts
multiple pools.
_Avoid_: pool contract, interaction address, vault address

**Pool ID**:
The bytes32 identifier used by vault-style protocols (Balancer V2/V3, UniV4) to
distinguish individual pools inside a singleton Pool Address. Empty/null for
UniV2/V3-style pools.
_Avoid_: pool key, pool hash

**TVL Pricing**:
The method used to convert pool reserves into USD for ranking Liquid Pools.
Stablecoins are pegged at $1; major volatile assets (WETH, WBTC) are priced via
on-chain reference pools. External price oracles are not required for ranking.
_Avoid_: valuation, market cap

**Route**:
A multi-hop path from token A to token B through one or more Liquid Pools, e.g.
A→B, A→C→B, or A→C→D→B. Ranked by total output (descending), then total fee
(ascending), then minimum pool TVL (descending), then quote confidence
(Exact > Estimated), then hop count (ascending). Routes whose `total_output`
could not be computed (no `amount_in` supplied, or a hop's quoter returned
`None`) sort after every quoted route. Intermediate tokens are restricted to a
whitelist of major liquid tokens.
_See also_: Pool, Liquid Pool, Pool Snapshot, Quote Confidence.
_Avoid_: path, swap path

**Router**:
A pass-through contract that receives token A and forwards token A elsewhere (same
token pass-through). Not a pool — identified by fund-flow analysis in classifier.
_Avoid_: aggregator

**Lending Market**:
A deposit/borrow market in a lending protocol (e.g., Aave, Compound). Distinct
from a Liquid Pool: depositing collateral locks one token while borrowing
another creates an obligation, not a final swap. Tracked separately for
liquidation and collateral-swap MEV strategies, not included in the V1 routing
graph.
_Avoid_: reserve pool, lending pool

**Bootstrap File**:
An optional JSON file containing a curated list of well-known pools. Read by
`mev-ant seed-pools` before the TheGraph path runs, so the registry can be
primed without depending on TheGraph availability. The bootstrap is additive
(`ON CONFLICT DO NOTHING`), so re-running with an updated file is safe. The
`version` field is a positive integer; the loader rejects unknown versions.
The bootstrap does not bypass TheGraph — it supplements it. A daily refresh
in the Liquidity Job still re-seeds from TheGraph to keep the registry current.
_See also_: Liquid Pool, Liquidity Job.
_Avoid_: snapshot, seed file

**Pool Snapshot**:
A point-in-time record of a pool's reserves, prices, and derived TVL. Only the
latest snapshot per pool is retained for routing. Snapshots are produced per
block for pools touched by state-changing events (e.g., Swap, Mint, Burn, Sync),
with a daily full refresh of all Liquid Pools synchronized with the TheGraph
re-seed. The `observed_at_block` field records the block at which the snapshot
was read; the snapshot is not per-block history.
_See also_: Pool, Liquid Pool.
_Avoid_: reserve sample, pool state

**Swap Event**:
A DEX-specific emitted log recording a swap. Used for pool discovery via classifier.
Currently supports 13 DEX families: Uniswap V2/V3/V4, Curve Vyper/Router,
Balancer V2/V3, DODO, Maverick V1/V2, Ekubo, LiquidityBook, Solidly.
_Avoid_: trade event, exchange event

**Transfer Event**:
An ERC20 Transfer(address,address,uint256) log. The PRIMARY signal for sandwich
detection — all token flows are unified as Transfer events (including internal
transfers from call frame trace logs). Obtained via eth_dxgTraceBlockByNumber.
_Avoid_: token movement, token flow

**Trade Signature**:
Per-executor, per-tx net token deltas from pool-involved transfers only.
Positive = received from pool, negative = paid to pool. Used for reversal matching.
_Avoid_: flow delta, token net

**Profit**:
Aggregated multi-token net flow for supported tokens:
Σ(back_deltas - front_deltas) for WETH, USDT, USDC, DAI, WBTC. Other tokens
ignored. Computed per-token (front deltas summed, back deltas summed, then compared).
_Avoid_: gain, earnings

**Cost**:
Attacker total spend: Σ gas_used × (base_fee + priority_fee) + Σ direct ETH transfers
to coinbase from attacker roles (attacker, executor, initiator). All costs in ETH.
_Avoid_: expense, spend

**Coinbase Income**:
What block.coinbase earns from this sandwich: Σ priority fees + direct ETH bribes.
_Avoid_: validator revenue, builder fee

**Net**:
Profit - Cost = attacker net gain (signed, ETH-denominated). Negative = loss.
_Avoid_: revenue, realized profit, pure profit

**Victim**:
An intermediate tx between frontrun and backrun whose sender is not the attacker,
executor, or initiator. Must trade on the same pool as the front executor with
the same trade direction (pays same token, receives same token).
_Avoid_: prey, target tx

**Supported Token**:
Tokens used for profit calculation and victim detection: WETH, USDC, USDT, DAI, WBTC.
Victim pool involvement checks use ALL tokens from Transfer events, not just supported ones.
_Avoid_: recognized token

**Quote Confidence**:
A label on a Route indicating whether the on-chain output can be computed
exactly. `Exact` means every hop has a quoter that knows the math (UniV2,
UniV3, Curve, SushiSwap, FraxSwap, PancakeSwap V3). `Estimated` means at
least one hop uses a quoter that doesn't know the exact math (Balancer V2/V3,
Fluid DEX in V1); the route's `total_output` is a rough estimate and may
diverge from realised output. Routes that cannot be quoted at all
(`total_output = None`) sort after every quoted route.
_See also_: Route.
_Avoid_: quote type, confidence level

**Unit Test**:
A fast, deterministic test that runs offline without external infrastructure. It targets
a single module or seam (classifier, funder, post-process) using in-memory fixtures.
_Avoid_: module test, internal test

**Integration Test**:
A regression test that exercises the full sandwich-detection pipeline against a live
Reth node on a real mainnet block. It requires `MEV_ANT_RPC_URL` (or the default RPC)
to be reachable and fails loudly otherwise.
_Avoid_: end-to-end test, regression test (ambiguous)

## Detection Algorithm

### Step 0 — Classification
Receipt-level logs only (not internal frames). Classify addresses:
- Swap event → Pool (overrides any prior Token classification)
- Transfer/Approval event → Token (only if not already Pool)
- Blacklist → Infra
- Fund-flow analysis: same-token pass-through → Router; different-token exchange → Pool
- Remaining → Unknown (candidates for executor/funder)

Output: `pool_or_router` set, `unknown` set, `kinds` map.

### Stage 1 — Filter
Keep only txs with ≥2 Transfer events. Nonce fillers and approvals-only txs excluded.

### Stage 2 — Executor Discovery
Per filtered tx, per Unknown address:
- Pool-involved transfers → `exec_deltas` (per-token net) + `exec_pools` (touched pools)
- Empty-delta stubs for `tx.from`/`tx.to` if not already tracked (enables pairing by initiator/target)

Output: `Vec<ExecutorTrade { tx_index, executor, deltas, pools, from, to }>`.

### Stage 3 — Tx-level Pairing
Group executor trades by tx_index. Pair txs by same initiator/target.
Aggregate ALL executor deltas in both txs. Best executor selected by
delta count. Handles single-executor and multi-contract patterns.
Calls `try_build_bundle` which runs: is_consecutive, share_pool, is_reversal,
profit aggregation, funder tracing, flashloan detection, victim identification,
cost computation.

### Stage 4 — Post-process
- `dedup_bundles`: per (front, back), keep highest profit
- `validate_bundles`: executor pool presence, same-pool recheck, funder consistency,
     profit recomputation (fallback when empty), victim revalidation, transfer collection
- `filter_bundles`: pool/funder blacklist, victim role filter, drop zero-victim bundles
- `resolve_overlaps`: non-overlapping, highest-profit

### Victim Identification
Victims must satisfy all:
- Tx between frontrun and backrun
- Sender ≠ initiator, attacker, executor
- Target ≠ front target (not attacker's own workers)
- Trade direction matches front executor (any token)
- Shares pool with front tx

### Roles (derived)
- **Attacker** = funder (from transfer graph)
- **Executor** = pool-touching address (from trade signature)
- **Initiator** = tx.from of frontrun
- **Funder** = capital source traced from front tx transfers
