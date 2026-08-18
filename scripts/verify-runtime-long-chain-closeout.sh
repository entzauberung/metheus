#!/usr/bin/env bash
set -euo pipefail

# Run the deterministic six-task fixture only. The fixture owns all fault
# injection and assertions; this wrapper only bounds execution and audits the
# structured facts emitted by the target test.
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
TARGET_TIMEOUT_SECONDS=300
CORE_MAX_JOBS=${CARGO_BUILD_JOBS:-2}
WORK_DIR=$(mktemp -d "${TMPDIR:-/tmp}/metheus-runtime-long-chain.XXXXXX")

cleanup() {
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

before_status="$WORK_DIR/status.before"
after_status="$WORK_DIR/status.after"
fixture_output="$WORK_DIR/fixture.output"
git -C "$ROOT_DIR" status --porcelain=v1 --untracked-files=all >"$before_status"

case "$CORE_MAX_JOBS" in
  1|2) ;;
  *)
    printf '%s\n' 'runtime long-chain requires CARGO_BUILD_JOBS=1 or 2' >&2
    exit 1
    ;;
esac

run_target() {
  local label=$1
  shift
  printf '[runtime-long-chain] %s\n' "$label"
  timeout --signal=TERM --kill-after=10s "${TARGET_TIMEOUT_SECONDS}s" "$@"
}

run_target "deterministic six-task fixture" bash -c '
  cd "$1"
  CARGO_BUILD_JOBS="$2" cargo test --locked --offline --package metheus --lib \
    --no-default-features --profile core-dev runtime_long_chain_fixture:: \
    -- --nocapture --test-threads=1
' _ "$ROOT_DIR/src-tauri" "$CORE_MAX_JOBS" 2>&1 | tee "$fixture_output"

for task_id in T1 T2 T3 T4 T5 T6; do
  if ! grep -Fq "\"task_id\":\"$task_id\"" "$fixture_output"; then
    printf 'missing structured fact for %s\n' "$task_id" >&2
    exit 1
  fi
done

fact_count=$(grep -o 'LONG_CHAIN_TASK_FACT ' "$fixture_output" | wc -l)
if [[ "$fact_count" -ne 6 ]]; then
  printf 'expected 6 structured task facts, found %s\n' "$fact_count" >&2
  exit 1
fi
if ! grep -Fq 'LONG_CHAIN_SUMMARY ' "$fixture_output"; then
  printf '%s\n' 'missing structured long-chain summary' >&2
  exit 1
fi

git -C "$ROOT_DIR" status --porcelain=v1 --untracked-files=all >"$after_status"
if ! cmp -s "$before_status" "$after_status"; then
  printf '%s\n' 'runtime long-chain fixture changed the worktree' >&2
  diff -u "$before_status" "$after_status" >&2 || true
  exit 1
fi

printf '%s\n' 'runtime long-chain fixture: PASS'
