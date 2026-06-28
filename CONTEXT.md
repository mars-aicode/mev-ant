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
The EOA (`tx.from`) of the frontrun transaction. Recorded as `initiator`.
The backrun may have a different `tx.from`, recorded as `back_initiator`.
The two transactions are tied together by sharing the same funder (attacker),
regardless of whether their targets match. A mismatch between `initiator`
and `back_initiator` does not disqualify a sandwich bundle.
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
(NOT YET IMPLEMENTED — always empty in current detector output.)
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
A deposit/borrow market in a lending protocol (e.g., Aave, Compound). Two roles:

- **Detection**: Used in funder resolution to distinguish borrow/repay actions from
  direct capital provision. Tokens sent TO the executor from a lending protocol
  represent a borrow, not a funder; tokens sent FROM the executor to a lending
  protocol represent a repay or deposit, not a pool-out. Without `lending_set`,
  the funder resolver would misattribute Aave/Compound as the capital source.
- **Routing**: NOT included in the V1 routing graph. Deposit/borrow creates a debt
  obligation rather than a final swap. Tracked separately for liquidation and
  collateral-swap MEV strategies in a future version.

Distinct from a Liquid Pool.
_Avoid_: reserve pool, lending pool

**Liquidity Job**:
A background task spawned by `mev-ant serve` that maintains the pool registry.
Two phases per tick:

- **Incremental**: processes blocks since the last cursor, fetching pool state
  (reserves, TVL) via `eth_call` for any pool touched by state-changing events
  (Swap, Mint, Burn, Sync) in those blocks. Only touched pools are updated.
- **Daily full refresh**: once per day at the configured hour, re-scans all
  registered pools from on-chain event data to discover new pools, re-ranks
  the top 1,000 by TVL into `liquid_pools`, and snapshots their latest state.

The job tolerates RPC failures and restarts: if the DB already contains registry
data, the job picks up from the last cursor and retries on the next tick.
_See also_: Liquid Pool, Pool Snapshot, Bootstrap File.
_Avoid_: refresher, background worker

**Bootstrap File**:
An optional JSON file containing a curated list of well-known pools. Read by
`mev-ant seed-pools` to prime the registry without depending on on-chain event
scanning. The bootstrap is additive (`ON CONFLICT DO NOTHING`), so re-running
with an updated file is safe. The `version` field is a positive integer; the
loader rejects unknown versions. The bootstrap does not bypass the Liquidity
Job — it supplements it. A daily full refresh in the Liquidity Job still
re-scans from on-chain event data to keep the registry current.
_See also_: Liquid Pool, Liquidity Job.
_Avoid_: snapshot, seed file

**Pool Snapshot**:
A point-in-time record of a pool's reserves, prices, and derived TVL, fetched
on-chain via `eth_call`. Only the latest snapshot per pool is retained for
routing. Snapshots are produced per block for pools touched by state-changing
events (e.g., Swap, Mint, Burn, Sync), with a daily full refresh of all Liquid
Pools synchronised with the Liquidity Job's event re-scan. The `observed_at_block`
field records the block at which the snapshot was read; the snapshot is not
per-block history.
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
Aggregated multi-token net flow for Supported Tokens:
Σ(front_deltas + back_deltas), keeping only entries where the net is positive.
Both front_deltas and back_deltas are signed (paying to pool = negative, receiving
from pool = positive), so a profitable sandwich has at least one Supported Token
where front outflows are offset by larger back inflows. Profit is a vector of
(token, amount) pairs, one per profitable Supported Token.
ETH and WETH are treated as a single economic unit for profit accounting.
_Avoid_: gain, earnings

**Cost**:
Attacker total spend: Σ gas_used × (base_fee + priority_fee) + Σ direct ETH transfers
to coinbase from attacker roles (attacker, executor, initiator). All costs in ETH.
_Avoid_: expense, spend

**Coinbase Income**:
What block.coinbase earns from this sandwich: Σ priority fees + direct ETH bribes.
_Avoid_: validator revenue, builder fee

**Net**:
The WETH/ETH portion of Profit minus Cost: `net = weth_profit - expense_wei`.
Expressed in wei (ETH-denominated). Only the WETH/ETH component is used because
Cost is always in ETH (gas + bribes); subtracting cost from USDC or USDT profit
would mix units. Net can be negative even when Profit has positive entries in
non-ETH tokens. Negative = attacker lost ETH after costs, regardless of
non-ETH token gains.
_Avoid_: revenue, realized profit, pure profit

**Victim**:
An intermediate tx between frontrun and backrun whose sender is not the attacker,
executor, or initiator. Must trade on the same pool as the front executor with
the same trade direction (pays same token, receives same token).
_Avoid_: prey, target tx

**Supported Token**:
Tokens used for profit calculation and victim detection: ETH, WETH, USDC, USDT, DAI, WBTC.
ETH and WETH are treated as equivalent (WETH unwraps 1:1 to ETH). Victim pool
involvement checks use ALL tokens from Transfer events, not just supported ones.
_Avoid_: recognized token

**Quote Confidence**:
A label on a Route indicating whether the on-chain output can be computed
exactly. `Exact` means every hop has a quoter that knows the math (UniV2,
UniV3, Curve, SushiSwap, FraxSwap, PancakeSwap V3). `Estimated` means at
least one hop uses a quoter whose output is an approximation rather than the
pool's exact math (e.g. Balancer, Fluid, or any pool without a dedicated
quoter); the route's `total_output` is `None` because the output cannot be
computed from the hop that lacks a quoter. Routes that cannot be quoted at
all (`total_output = None` because no `amount_in` was supplied) likewise
use `Estimated` confidence.
_See also_: Route, Route Sort Mode.
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
