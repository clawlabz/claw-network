# Changelog

## [0.2.0] - 2026-03-19

### Added
- **On-chain Agent Score**: Five-dimension automatic reputation calculation (Activity, Uptime, Block Production, Economic, Platform Activity) with time decay
- **PlatformActivityReport** transaction type (tx=11): Third-party platforms can report agent activity on-chain (requires >= 50,000 CLAW stake)
- **Activity Statistics**: Per-epoch automatic tracking of transaction counts, contract deployments, gas consumed per address
- **Validator Uptime Tracking**: Sliding window signed-blocks tracking for validator reliability scoring
- **`claw_getAgentScore` RPC**: Query per-address Agent Score with dimension breakdown

### Changed
- Agent Score calculation: from attestation-based (subjective) to on-chain behavior-based (automated)
- Consensus weight formula now uses multi-dimensional Agent Score

### Deprecated
- `ReputationAttest` (tx type 4): Kept for backward compatibility but no longer contributes to Agent Score
