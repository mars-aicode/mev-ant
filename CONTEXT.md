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

**Liquidity Pool**:
Additional DEX pools used only by the attacker (not the victim) for routing or
entry/exit in multi-hop trades.
_Avoid_: auxiliary pool, routing pool

**Pool**:
The DEX contract or vault entity where swaps execute. For Uniswap-style pools,
pool = contract address. For Balancer/UniV4 vaults, pool = decoded pool ID from
event params. Discovered via classifier (receipt-level Swap event topic0 matching).
_Avoid_: market, pair, venue

**Router**:
A pass-through contract that receives token A and forwards token A elsewhere (same
token pass-through). Not a pool — identified by fund-flow analysis in classifier.
_Avoid_: aggregator

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
