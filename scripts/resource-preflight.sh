#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
readonly TRACK="${1:-core}"

readonly CORE_MIN_DISK_KIB=4194304
readonly CORE_MIN_MEMORY_KIB=2097152
readonly GROK_MIN_DISK_KIB=8388608
readonly GROK_MIN_MEMORY_KIB=6291456
readonly GROK_ADAPTIVE_MIN_MEMORY_KIB=4194304
readonly GROK_ADAPTIVE_STANDARD_MEMORY_KIB=6291456
readonly DESKTOP_META_SUFFIX=".build-meta"
readonly TEST_MODE="${METHEUS_PREFLIGHT_TEST_MODE:-0}"

case "${TRACK}" in
  core|runtime-fault)
    readonly MIN_DISK_KIB="${CORE_MIN_DISK_KIB}"
    readonly MIN_MEMORY_KIB="${CORE_MIN_MEMORY_KIB}"
    ;;
  desktop)
    readonly MIN_DISK_KIB="${CORE_MIN_DISK_KIB}"
    readonly MIN_MEMORY_KIB="${CORE_MIN_MEMORY_KIB}"
    ;;
  grok)
    readonly MIN_DISK_KIB="${GROK_MIN_DISK_KIB}"
    readonly MIN_MEMORY_KIB="${GROK_MIN_MEMORY_KIB}"
    ;;
  grok-adaptive)
    readonly MIN_DISK_KIB="${GROK_MIN_DISK_KIB}"
    readonly MIN_MEMORY_KIB="${GROK_ADAPTIVE_MIN_MEMORY_KIB}"
    ;;
  *)
    echo "资源预检失败：未知验证轨道 ${TRACK}。" >&2
    exit 2
    ;;
esac

if [[ "${TEST_MODE}" == "1" ]]; then
  AVAILABLE_DISK_KIB="${METHEUS_PREFLIGHT_TEST_DISK_KIB-}"
  AVAILABLE_MEMORY_KIB="${METHEUS_PREFLIGHT_TEST_MEMORY_KIB-}"
else
  AVAILABLE_DISK_KIB="$(df -Pk "${REPO_ROOT}" | awk 'NR == 2 { print $4 }')"
  AVAILABLE_MEMORY_KIB="$(awk '/^MemAvailable:/ { print $2; exit }' /proc/meminfo 2>/dev/null || true)"
fi

if [[ "${TRACK}" == "grok" || "${TRACK}" == "grok-adaptive" ]]; then
  if [[ "${TEST_MODE}" == "1" ]]; then
    CARGO_PRESENT="${METHEUS_PREFLIGHT_TEST_CARGO_PRESENT:-0}"
    if [[ "${CARGO_PRESENT}" == "1" ]]; then
      echo "R2_RESOURCE_PROFILE=HARD_STOP reason=检测到其他 Cargo 进程" >&2
      echo "资源预检失败：检测到其他 Cargo 进程，拒绝启动 Grok 检查。" >&2
      exit 1
    fi
  elif pgrep -x cargo >/dev/null 2>&1; then
    echo "资源预检失败：检测到其他 Cargo 进程，拒绝启动 Grok 检查。" >&2
    exit 1
  fi
fi

if [[ ! "${AVAILABLE_DISK_KIB}" =~ ^[0-9]+$ ]]; then
  if [[ "${TRACK}" == "grok-adaptive" ]]; then
    echo "R2_RESOURCE_PROFILE=HARD_STOP reason=无法读取可用磁盘空间" >&2
  fi
  echo "资源预检失败：无法读取可用磁盘空间。" >&2
  exit 1
fi
if (( AVAILABLE_DISK_KIB < MIN_DISK_KIB )); then
  if [[ "${TRACK}" == "grok-adaptive" ]]; then
    echo "R2_RESOURCE_PROFILE=HARD_STOP reason=可用磁盘空间不足" >&2
  fi
  echo "资源预检失败：${TRACK} 轨道至少需要 ${MIN_DISK_KIB} KiB 可用磁盘，当前 ${AVAILABLE_DISK_KIB} KiB。" >&2
  exit 1
fi

if [[ "${TRACK}" == "grok-adaptive" ]]; then
  if [[ ! "${AVAILABLE_MEMORY_KIB}" =~ ^[0-9]+$ ]]; then
    echo "R2_RESOURCE_PROFILE=HARD_STOP reason=无法读取或解析可用内存" >&2
    echo "资源预检失败：无法解析自适应 Grok 轨道的可用内存。" >&2
    exit 1
  fi
  if (( AVAILABLE_MEMORY_KIB < MIN_MEMORY_KIB )); then
    echo "R2_RESOURCE_PROFILE=HARD_STOP reason=可用内存低于 ${MIN_MEMORY_KIB} KiB" >&2
    echo "资源预检失败：grok-adaptive 轨道至少需要 ${MIN_MEMORY_KIB} KiB 可用内存，当前 ${AVAILABLE_MEMORY_KIB} KiB。" >&2
    exit 1
  fi
  if (( AVAILABLE_MEMORY_KIB >= GROK_ADAPTIVE_STANDARD_MEMORY_KIB )); then
    R2_RESOURCE_PROFILE="STANDARD"
  else
    R2_RESOURCE_PROFILE="CONSTRAINED"
  fi
elif [[ -n "${AVAILABLE_MEMORY_KIB}" ]]; then
  if [[ ! "${AVAILABLE_MEMORY_KIB}" =~ ^[0-9]+$ ]]; then
    echo "资源预检失败：无法解析可用内存。" >&2
    exit 1
  fi
  if (( AVAILABLE_MEMORY_KIB < MIN_MEMORY_KIB )); then
    echo "资源预检失败：${TRACK} 轨道至少需要 ${MIN_MEMORY_KIB} KiB 可用内存，当前 ${AVAILABLE_MEMORY_KIB} KiB。" >&2
    exit 1
  fi
fi

if [[ "${TRACK}" == "grok-adaptive" ]]; then
  echo "资源预检通过：track=${TRACK} disk_kib=${AVAILABLE_DISK_KIB} memory_kib=${AVAILABLE_MEMORY_KIB} R2_RESOURCE_PROFILE=${R2_RESOURCE_PROFILE}"
else
  echo "资源预检通过：track=${TRACK} disk_kib=${AVAILABLE_DISK_KIB} memory_kib=${AVAILABLE_MEMORY_KIB:-unknown}"
fi

if [[ "${TRACK}" == "runtime-fault" ]]; then
  readonly FAULT_TEMP_PARENT="${TMPDIR:-/tmp}"
  if [[ ! -d "${FAULT_TEMP_PARENT}" || ! -w "${FAULT_TEMP_PARENT}" ]]; then
    echo "资源预检失败：运行期故障轨道需要可写临时目录 ${FAULT_TEMP_PARENT}。" >&2
    exit 1
  fi
  AVAILABLE_TEMP_DISK_KIB="$(df -Pk "${FAULT_TEMP_PARENT}" | awk 'NR == 2 { print $4 }')"
  if [[ ! "${AVAILABLE_TEMP_DISK_KIB}" =~ ^[0-9]+$ ]] \
    || (( AVAILABLE_TEMP_DISK_KIB < CORE_MIN_DISK_KIB )); then
    echo "资源预检失败：运行期故障临时目录至少需要 ${CORE_MIN_DISK_KIB} KiB 可用空间。" >&2
    exit 1
  fi
  FAULT_TEMP_PROBE="$(mktemp -d "${FAULT_TEMP_PARENT}/metheus-runtime-preflight.XXXXXX")"
  rmdir -- "${FAULT_TEMP_PROBE}"
  echo "RUNTIME_FAULT_INJECTION_ELIGIBLE=yes temp_parent=${FAULT_TEMP_PARENT}"
fi

if [[ "${TRACK}" == "desktop" ]]; then
  mapfile -t DESKTOP_SOURCE_FILES < <(
    {
      rg --files "${REPO_ROOT}/src" "${REPO_ROOT}/src-tauri/src"
      for file in \
        "${REPO_ROOT}/index.html" \
        "${REPO_ROOT}/package.json" \
        "${REPO_ROOT}/package-lock.json" \
        "${REPO_ROOT}/vite.config.ts" \
        "${REPO_ROOT}/src-tauri/build.rs" \
        "${REPO_ROOT}/src-tauri/Cargo.toml" \
        "${REPO_ROOT}/src-tauri/Cargo.lock" \
        "${REPO_ROOT}/src-tauri/tauri.conf.json"; do
        [[ -f "${file}" ]] && echo "${file}"
      done
    } | sort -u
  )
  if (( ${#DESKTOP_SOURCE_FILES[@]} == 0 )); then
    echo "DESKTOP_SMOKE_ELIGIBLE=no reason=无法枚举桌面源码" >&2
    exit 3
  fi
  SOURCE_FINGERPRINT="$(sha256sum "${DESKTOP_SOURCE_FILES[@]}" | sha256sum | awk '{ print $1 }')"
  readonly SOURCE_FINGERPRINT
  readonly CORE_DESKTOP_CANDIDATES=(
    "${REPO_ROOT}/.build/core/core-dev/metheus"
    "${REPO_ROOT}/.build/core/debug/metheus"
  )

  for candidate in "${CORE_DESKTOP_CANDIDATES[@]}"; do
    [[ -x "${candidate}" ]] || continue
    metadata="${candidate}${DESKTOP_META_SUFFIX}"
    if [[ ! -f "${metadata}" ]]; then
      echo "桌面候选缺少构建元数据：${candidate}" >&2
      continue
    fi
    if ! rg -q '^track=core$' "${metadata}" \
      || ! rg -q '^default_features=false$' "${metadata}" \
      || ! rg -q "^source_fingerprint=${SOURCE_FINGERPRINT}$" "${metadata}"; then
      echo "桌面候选的 Core 特性或源码指纹不匹配：${candidate}" >&2
      continue
    fi
    if find "${REPO_ROOT}/src" "${REPO_ROOT}/src-tauri/src" \
      -type f -newer "${candidate}" -print -quit | rg -q .; then
      echo "桌面候选早于当前源码：${candidate}" >&2
      continue
    fi
    echo "DESKTOP_SMOKE_ELIGIBLE=yes binary=${candidate} source_fingerprint=${SOURCE_FINGERPRINT}"
    exit 0
  done

  if [[ -x "${REPO_ROOT}/src-tauri/target/debug/metheus" ]]; then
    echo "非 Core 候选不具备验收资格：${REPO_ROOT}/src-tauri/target/debug/metheus" >&2
  fi
  echo "DESKTOP_SMOKE_ELIGIBLE=no reason=没有同时满足 Core 路径、特性元数据、源码指纹与时间戳的桌面二进制" >&2
  exit 3
fi
