#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly PREFLIGHT="${SCRIPT_DIR}/resource-preflight.sh"
readonly TEST_DISK_KIB=16777216

assert_contains() {
  local output="$1"
  local expected="$2"
  local label="$3"
  if [[ "${output}" != *"${expected}"* ]]; then
    printf 'FAIL %s: expected output to contain %q\n%s\n' "${label}" "${expected}" "${output}" >&2
    exit 1
  fi
}

run_adaptive_case() {
  local label="$1"
  local memory_kib="$2"
  local expected_status="$3"
  local expected_profile="$4"
  local disk_kib="${5:-${TEST_DISK_KIB}}"
  local cargo_present="${6:-0}"
  local output
  local status

  set +e
  output="$(
    METHEUS_PREFLIGHT_TEST_MODE=1 \
    METHEUS_PREFLIGHT_TEST_MEMORY_KIB="${memory_kib}" \
    METHEUS_PREFLIGHT_TEST_DISK_KIB="${disk_kib}" \
    METHEUS_PREFLIGHT_TEST_CARGO_PRESENT="${cargo_present}" \
    "${PREFLIGHT}" grok-adaptive 2>&1
  )"
  status=$?
  set -e

  if (( status != expected_status )); then
    printf 'FAIL %s: expected exit %d, got %d\n%s\n' "${label}" "${expected_status}" "${status}" "${output}" >&2
    exit 1
  fi
  assert_contains "${output}" "R2_RESOURCE_PROFILE=${expected_profile}" "${label} profile"
  printf 'PASS %s\n' "${label}"
}

run_legacy_grok_case() {
  local label="$1"
  local memory_kib="$2"
  local expected_status="$3"
  local output
  local status

  set +e
  output="$(
    METHEUS_PREFLIGHT_TEST_MODE=1 \
    METHEUS_PREFLIGHT_TEST_MEMORY_KIB="${memory_kib}" \
    METHEUS_PREFLIGHT_TEST_DISK_KIB="${TEST_DISK_KIB}" \
    METHEUS_PREFLIGHT_TEST_CARGO_PRESENT=0 \
    "${PREFLIGHT}" grok 2>&1
  )"
  status=$?
  set -e

  if (( status != expected_status )); then
    printf 'FAIL %s: expected exit %d, got %d\n%s\n' "${label}" "${expected_status}" "${status}" "${output}" >&2
    exit 1
  fi
  if (( expected_status == 0 )); then
    assert_contains "${output}" "track=grok" "${label} track"
  else
    assert_contains "${output}" "grok 轨道至少需要 6291456 KiB" "${label} threshold"
  fi
  if [[ "${output}" == *"R2_RESOURCE_PROFILE="* ]]; then
    printf 'FAIL %s: legacy grok track emitted adaptive profile\n%s\n' "${label}" "${output}" >&2
    exit 1
  fi
  printf 'PASS %s\n' "${label}"
}

run_adaptive_case "adaptive lower hard stop" 4194303 1 HARD_STOP
run_adaptive_case "adaptive lower boundary" 4194304 0 CONSTRAINED
run_adaptive_case "adaptive constrained upper boundary" 6291455 0 CONSTRAINED
run_adaptive_case "adaptive standard boundary" 6291456 0 STANDARD
run_adaptive_case "adaptive unknown memory hard stop" unknown 1 HARD_STOP
run_adaptive_case "adaptive disk shortage hard stop" 6291456 1 HARD_STOP 8388607
run_adaptive_case "adaptive cargo conflict hard stop" 6291456 1 HARD_STOP "${TEST_DISK_KIB}" 1
run_legacy_grok_case "legacy grok below six GiB" 6291455 1
run_legacy_grok_case "legacy grok six GiB boundary" 6291456 0

printf 'PASS resource preflight profile boundaries\n'
