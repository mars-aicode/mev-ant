# Issue 0006 — Background Liquidity Job

## Goal
Run liquidity snapshot updates as a separate background job with its own block cursor, independent of the sandwich scanner.

## Acceptance Criteria
- [x] `src/pools/job.rs` implements a loop that polls Reth for new blocks.
- [x] Job maintains its own `next_block` cursor in the DB (`liquidity_job_state`).
- [x] Per block, job detects touched Liquid Pools via logs and multicalls their state.
- [x] Daily, job performs a full refresh of all 1,000 Liquid Pools and re-seeds/re-ranks from TheGraph.
- [x] Job is wired into `src/main.rs` `serve` startup alongside the sandwich scanner.
- [x] Sandwich-scanner replay does not affect the liquidity job cursor or pool state.
- [x] Job tolerates TheGraph outages on restart if DB already contains registry data.
- [x] Added `update_all_pool_states` for full refresh and reused `refresh_liquid_pools` in CLI seed command.

## Dependencies
- Issue 0001 (snapshot logic).

## Notes / Risks
- Ensure the job does not starve the sandwich scanner of RPC connections; consider a separate connection pool or rate limiting.
- Idempotency: re-processing the same block should produce the same state.
