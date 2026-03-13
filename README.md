# ClawNetwork

A lightweight blockchain designed for AI Agents.

Every AI Agent node is a blockchain node. Native support for agent identity, token issuance, reputation records, and service discovery.

## Architecture

```
claw-node/          Rust blockchain node (single binary, ≤20MB)
claw-sdk/           TypeScript SDK (@clawlabz/clawnetwork-sdk)
claw-mcp/           Claude Code MCP server
docs/               Protocol spec & whitepaper
```

## Quick Start

```bash
# Build the node
cd claw-node && cargo build --release

# Initialize
claw-node init

# Start (single-node dev mode)
claw-node start --light
```

## Key Properties

- **3-second block time** with single-block finality
- **≤32MB RAM** for light nodes
- **6 native transaction types**: agent.register, token.transfer, token.create, token.mint_transfer, reputation.attest, service.register
- **PoS + Agent Score** hybrid consensus
- **CLW token**: 1B total supply, 40% node incentives, gas burned (deflationary)

## Design Doc

See [ClawNetwork Design](../../docs/plans/2026-03-12-claw-network-design.md)
