#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
readonly TRACK="${1:-core}"

readonly CORE_MIN_DISK_KIB=4194304
readonly CORE_MIN_MEMORY_KIB=2097152
readonly GROK_MIN_DISK_KIB=8388608
readonly GROK_MIN_MEMORY_KIB=6291456
readonly DESKTOP_META_SUFFIX=".build-meta"

case "${TRACK}" in
  core)
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
    if pgrep -x cargo >/dev/null 2>&1; then
      echo "资源预检失败：检测到其他 Cargo 进程，拒绝启动 Grok 检查。" >&2
      exit 1
    fi
    ;;
  *)
    echo "资源预检失败：未知验证轨道 ${TRACK}。" >&2
    exit 2
    ;;
esac

AVAILABLE_DISK_KIB="$(df -Pk "${REPO_ROOT}" | awk 'NR == 2 { print $4 }')"
AVAILABLE_MEMORY_KIB="$(awk '/^MemAvailable:/ { print $2; exit }' /proc/meminfo 2>/dev/null || true)"

if [[ ! "${AVAILABLE_DISK_KIB}" =~ ^[0-9]+$ ]]; then
  echo "资源预检失败：无法读取可用磁盘空间。" >&2
  exit 1
fi
if (( AVAILABLE_DISK_KIB < MIN_DISK_KIB )); then
  echo "资源预检失败：${TRACK} 轨道至少需要 ${MIN_DISK_KIB} KiB 可用磁盘，当前 ${AVAILABLE_DISK_KIB} KiB。" >&2
  exit 1
fi

if [[ -n "${AVAILABLE_MEMORY_KIB}" ]]; then
  if [[ ! "${AVAILABLE_MEMORY_KIB}" =~ ^[0-9]+$ ]]; then
    echo "资源预检失败：无法解析可用内存。" >&2
    exit 1
  fi
  if (( AVAILABLE_MEMORY_KIB < MIN_MEMORY_KIB )); then
    echo "资源预检失败：${TRACK} 轨道至少需要 ${MIN_MEMORY_KIB} KiB 可用内存，当前 ${AVAILABLE_MEMORY_KIB} KiB。" >&2
    exit 1
  fi
fi

echo "资源预检通过：track=${TRACK} disk_kib=${AVAILABLE_DISK_KIB} memory_kib=${AVAILABLE_MEMORY_KIB:-unknown}"

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
