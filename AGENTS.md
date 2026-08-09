# Continuous Blueprint Execution

## Source Of Truth

- The execution state is `ai/STATE.md`.
- The global constraints are in `ai/CONTROL.md`.
- The execution protocol is in `ai/EXECUTION-PROTOCOL.md`.
- The current work order is in the WO file selected by `ai/STATE.md`.
- The current subtask is in the ST file selected by `ai/STATE.md`.
- The latest result is in `ai/LATEST-REPORT.md`.

## Startup

Read these files in this order:

1. `AGENTS.md`
2. `ai/CONTROL.md`
3. `ai/STATE.md`
4. `ai/EXECUTION-PROTOCOL.md`
5. The current WO file.
6. The current ST file.

Do not read every future WO or ST file. Load only the current task selected by `ai/STATE.md`.

## Continuous Execution

- Execute the current ST.
- Do not wait for confirmation after a successful ST.
- After a successful ST, run only its listed targeted verification.
- Write `ai/LATEST-REPORT.md`.
- Update `ai/STATE.md`.
- Read the next ST selected by the updated state.
- Continue automatically.
- After all STs in the current WO pass, continue to the next dependency-ready WO.
- Do not stop merely because one ST has finished.
- Do not start tasks outside the blueprint.

## Scope

- Modify only files allowed by the current ST.
- Never modify files outside the allowlist.
- Never change dependencies, lock files, data models, architecture, or external repositories unless the current ST explicitly permits it.
- Never commit or push.
- Never start a second agent or parallel implementation process.

## Resource Guard

- Never run a full build or full test suite by default.
- Never run `npm install`, `npm ci`, `pnpm install`, `cargo clean`, `tauri build`, Docker builds, release builds, workspace-wide checks, watch mode, or development servers.
- Never use `npm test`, `npm run build`, `cargo test`, `cargo build`, or `cargo check --workspace` unless the current ST explicitly lists the exact command and marks it as R2.
- R2 requires explicit user approval and must stop before execution.
- R1 allows only the exact targeted commands listed in the current ST.
- Run one command at a time.
- Never run build or test commands in parallel.
- For Rust commands, use `CARGO_BUILD_JOBS=2` when the current ST allows Rust verification.
- Stop if the process causes OOM, severe system lag, disk exhaustion, or is killed by the operating system.
- Stop if a command exceeds the maximum runtime written in the ST file.

## Repair Limit

- Maximum two edit -> verify -> repair cycles per ST.
- If the same failure occurs twice, mark the state `BLOCKED`.
- Do not broaden the command or test scope to hide a failure.
- Do not skip a failed ST to reach a later ST.

## State Update

After a successful ST, update `ai/STATE.md` with:

- `STATUS: RUNNING`
- completed WO/ST
- next WO/ST
- `LAST_REPORT`
- `LAST_UPDATED`

After a hard stop, update:

- `STATUS: BLOCKED`
- exact blocker
- failed command or missing prerequisite
- files changed
- recommended human decision

When all tasks pass, update:

- `STATUS: COMPLETED`

Always overwrite the current report and current state files. Do not create large cumulative logs.
