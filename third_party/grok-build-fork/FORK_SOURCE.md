# Metheus controlled Grok Build fork

- Upstream baseline: `7cfcb20d2b50b0d18801a6c0af2e401c0e060894`
- Upstream Git tree: `0043d955d5848e0275c36458fc63d5c06a804b9b`
- Fork revision: `metheus.2`
- Created: 2026-07-23
- License: Apache-2.0

This directory is a complete, repository-local fork of the pristine source
snapshot in `../grok-build`. The pristine snapshot remains the audit baseline;
Metheus-specific changes are listed in `PATCHSET.md`.

The fork exists because the upstream `MvpAgent` session construction and
`SessionActor` loop are crate-private. It exposes one feature-gated facade that
uses those real upstream components while replacing their host integrations
with a frozen Metheus configuration and a four-tool filesystem boundary.

The built-in engine never invokes the `grok` executable, resolves Grok CLI
credentials or model configuration, or starts Shell, `rg`, MCP, plugin, skill,
memory, hook, or subagent facilities. All modified paths and their rationale are
listed in `PATCHSET.md`.

## Baseline spot hashes

These hashes were captured immediately before the fork was created:

- `Cargo.toml`: `28d8adc6ced901075cdb0beb21219b468d2c8e77451954fc70db3fbc4adcc1b0`
- `Cargo.lock`: `b6f903c9654a1a99c6820a1de8111059af4f2b17097056d9fa7ee477c4c368f4`
- `xai-grok-shell/src/lib.rs`: `688082f70d8a8f9cd78a5db116326638a9f43f149eea1956ef67975100bfbdfb`
- `xai-grok-shell/session/spawn.rs`: `791c6ce7fa3aaa3dc6b533c7f409a40700cfe8adaedcb198d8fbed1b3ed6ffb4`
