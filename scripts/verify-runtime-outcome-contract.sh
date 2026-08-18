#!/usr/bin/env bash
set -euo pipefail

# This gate is intentionally a small composition of repository-owned checks.
# It does not install dependencies, contact a provider, or start a service.
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
TARGET_TIMEOUT_SECONDS=300
WORK_DIR=$(mktemp -d "${TMPDIR:-/tmp}/metheus-runtime-outcome.XXXXXX")

cleanup() {
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

before_status="$WORK_DIR/status.before"
after_status="$WORK_DIR/status.after"
git -C "$ROOT_DIR" status --porcelain=v1 --untracked-files=all >"$before_status"

run_target() {
  local label=$1
  shift
  printf '[runtime-outcome] %s\n' "$label"
  timeout --signal=TERM --kill-after=10s "${TARGET_TIMEOUT_SECONDS}s" "$@"
}

run_target "phase1 outcome and acceptance contract" \
  bash "$ROOT_DIR/scripts/verify-phase1-runtime-contract.sh"

git -C "$ROOT_DIR" status --porcelain=v1 --untracked-files=all >"$after_status"
if ! cmp -s "$before_status" "$after_status"; then
  printf '%s\n' 'runtime outcome gate changed the worktree' >&2
  diff -u "$before_status" "$after_status" >&2 || true
  exit 1
fi

printf '%s\n' 'runtime outcome contract: PASS'
