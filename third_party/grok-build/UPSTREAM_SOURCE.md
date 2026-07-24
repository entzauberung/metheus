# Grok Build upstream source

- Upstream working copy: `/home/bruce/grok-build` (provenance only; builds do not use this path)
- Git revision: `7cfcb20d2b50b0d18801a6c0af2e401c0e060894`
- Upstream `SOURCE_REV`: `f9736c7b86f8e1c0e99e20ebbbd1195cd0c147e3`
- Git tree: `0043d955d5848e0275c36458fc63d5c06a804b9b`
- `git archive` SHA-256: `0ea55325fa01a53e0d5a46c016630ae80c680026d8d62de7f0963af03fd473e8`
- Imported: 2026-07-23
- License: Apache-2.0
- Required Rust toolchain: 1.92
- Metheus adapter version: 1

The directory is an unmodified `git archive` of the revision above. The Git tree
object is the canonical per-file integrity manifest. Repository metadata, build
outputs, caches, untracked configuration, and local credentials are not included.

This pristine tree is retained only as the byte-for-byte source audit baseline.
Metheus links the separate controlled fork in `../grok-build-fork`, whose
feature-gated facade drives the upstream `MvpAgent`, `AgentBuilder`, and
`SessionActor` in-process. The built-in engine never invokes the Grok CLI.
