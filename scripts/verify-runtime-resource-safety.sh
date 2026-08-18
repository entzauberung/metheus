#!/usr/bin/env bash
set -euo pipefail

# Resource checks are bounded and synthetic. They never create memory pressure
# and leave process ownership to the existing repository-owned test gates.
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
TARGET_TIMEOUT_SECONDS=300
WORK_DIR=$(mktemp -d "${TMPDIR:-/tmp}/metheus-runtime-resource.XXXXXX")

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
  printf '[runtime-resource] %s\n' "$label"
  timeout --signal=TERM --kill-after=10s "${TARGET_TIMEOUT_SECONDS}s" "$@"
}

run_target "startup resource preflight" \
  bash "$ROOT_DIR/scripts/resource-preflight.sh"
run_target "bounded runtime fault and termination contract" \
  bash "$ROOT_DIR/scripts/verify-runtime-fault-recovery.sh"

git -C "$ROOT_DIR" status --porcelain=v1 --untracked-files=all >"$after_status"
if ! cmp -s "$before_status" "$after_status"; then
  printf '%s\n' 'runtime resource gate changed the worktree' >&2
  diff -u "$before_status" "$after_status" >&2 || true
  exit 1
fi

printf '%s\n' 'runtime resource safety: PASS'
