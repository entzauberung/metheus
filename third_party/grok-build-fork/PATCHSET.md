# Metheus fork patch set

Fork revision: `metheus.2`

The only supported embedded call chain is:

```text
metheus-grok-engine
  -> xai-grok-shell::metheus_embedded
  -> MvpAgent
  -> AgentBuilder
  -> SessionActor
  -> restricted FinalizedToolset
```

## Allowed changes

| Path | Purpose |
| --- | --- |
| `FORK_SOURCE.md` | Fork provenance, baseline identity, and operating boundary |
| `PATCHSET.md` | Audited patch inventory |
| `UPSTREAM_SOURCE.md` | Identify this directory as the controlled fork rather than the pristine archive |
| `crates/codegen/xai-file-utils/src/events/tracker.rs` | Add a no-op event tracker so embedded sessions never create `~/.grok` event files |
| `crates/codegen/xai-grok-sampler/src/client.rs` | Remove authorization and API-key prefix fields from sampler trace events; retain only non-secret authentication type/presence metadata |
| `crates/codegen/xai-grok-sampler/src/config.rs` | Redact API keys and extra header values from `SamplerConfig` debug output |
| `crates/codegen/xai-grok-sampler/src/sampling_log.rs` | Remove credential prefixes from sampling request spans so sampler tracing cannot persist partial API keys |
| `crates/codegen/xai-grok-shell/Cargo.toml` | Declare the opt-in `metheus-embedded` feature |
| `crates/codegen/xai-grok-shell/src/lib.rs` | Export the feature-gated embedded facade |
| `crates/codegen/xai-grok-shell/src/metheus_embedded.rs` | Build and drive the real upstream session actor with frozen Metheus config, bounded lifecycle, ACP policy client, and typed errors |
| `crates/codegen/xai-grok-shell/tests/metheus_embedded_runtime.rs` | Exercise the embedded SessionActor through local fake SSE without compiling the shell unit-test harness |
| `crates/codegen/xai-grok-shell/src/agent/config.rs` | Carry an explicit process-local embedded mode that cannot be loaded from Grok configuration |
| `crates/codegen/xai-grok-shell/src/agent/mvp_agent/acp_agent.rs` | Keep ordinary ACP session constructors explicit about not using embedded mode |
| `crates/codegen/xai-grok-shell/src/agent/mvp_agent/agent_ops.rs` | Construct embedded `MvpAgent` sessions and disable host config, plugins, credentials, feedback, MCP, LSP, memory, hooks, and subagents |
| `crates/codegen/xai-grok-shell/src/agent/mvp_agent/mod.rs` | Thread embedded mode and file policy through session spawn options |
| `crates/codegen/xai-grok-shell/src/agent/subagent/handle_request.rs` | Supply non-embedded defaults to the extended upstream spawn signature |
| `crates/codegen/xai-grok-shell/src/session/acp_types.rs` | Add non-persisted embedded and frozen-auth startup hints |
| `crates/codegen/xai-grok-shell/src/session/acp_session_impl/prompt_queue.rs` | Suppress host prompt-history writes in embedded sessions |
| `crates/codegen/xai-grok-shell/src/session/acp_session_impl/sampler_turn.rs` | Reconstruct requests solely from the frozen snapshot; disable model-catalog auth and bearer refresh paths |
| `crates/codegen/xai-grok-shell/src/session/acp_session_impl/session_setup.rs` | Suppress system prompt and chat-history persistence |
| `crates/codegen/xai-grok-shell/src/session/acp_session_impl/spawn.rs` | Build the restricted upstream agent, bind its in-memory workspace route, and suppress host services and persistence |
| `crates/codegen/xai-grok-shell/src/session/acp_session_impl/turn.rs` | Suppress host task-resume injection |
| `crates/codegen/xai-grok-tools/src/implementations/mod.rs` | Register the embedded policy resource module |
| `crates/codegen/xai-grok-tools/src/implementations/metheus_embedded.rs` | Enforce canonical project paths, exact write authorization, bounded list/read, and in-process grep |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/grep/mod.rs` | Route embedded grep to the in-process policy instead of `rg` |
| `crates/codegen/xai-grok-tools/src/implementations/grok_build/list_dir/mod.rs` | Route embedded directory listing through the project-root policy |
| `crates/codegen/xai-grok-tools/tests/metheus_embedded_no_process.rs` | Fail if the embedded filesystem policy introduces a process-spawning API |

No other fork file may differ from the pristine `third_party/grok-build`
baseline without first being added to this inventory with a security rationale.

## Frozen runtime boundary

- Advertised model tools are exactly `read_file`, `search_replace`, `list_dir`, and `grep`.
- `read_file` and `search_replace` are executed by the restricted ACP client.
- `list_dir` and `grep` use the same canonical project policy; grep is Rust-only.
- No Shell, terminal fallback, web tool, MCP, plugin, skill, memory, hook, or subagent is available.
- Model, endpoint, backend, API key, timeout, and maximum turns come only from the Metheus execution snapshot.
- Embedded requests do not read Grok CLI model credentials or add URL-derived Grok authentication headers.
- Cancellation, timeout, maximum turns, policy rejection, auth, quota, rate limit, network, protocol, and runtime failures remain typed across the adapter boundary.

Audit the inventory with:

```bash
diff -qr --exclude target --exclude .git third_party/grok-build third_party/grok-build-fork
```
