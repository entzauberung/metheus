# Grok Build controlled fork source

- Upstream working copy: `/home/bruce/grok-build` (provenance only; builds do not use this path)
- Git revision: `7cfcb20d2b50b0d18801a6c0af2e401c0e060894`
- Upstream `SOURCE_REV`: `f9736c7b86f8e1c0e99e20ebbbd1195cd0c147e3`
- Git tree: `0043d955d5848e0275c36458fc63d5c06a804b9b`
- `git archive` SHA-256: `0ea55325fa01a53e0d5a46c016630ae80c680026d8d62de7f0963af03fd473e8`
- Imported: 2026-07-23
- License: Apache-2.0
- Required Rust toolchain: 1.92
- Metheus fork revision: `metheus.2`
- Metheus adapter version: 2

This directory starts from the unmodified archive recorded above and contains
the controlled Metheus changes listed in `PATCHSET.md`. The pristine audit
baseline remains in `../grok-build`; its Git tree object is the canonical
per-file integrity manifest.

Metheus links `xai-grok-shell` with the opt-in `metheus-embedded` feature. The
facade drives the upstream `MvpAgent`, `AgentBuilder`, and `SessionActor` in the
application process. It does not invoke the Grok CLI. Its model-visible tools
are restricted to `read_file`, `search_replace`, `list_dir`, and `grep`, all
under a frozen project-root and exact write authorization policy.
