#!/usr/bin/env bash
set -euo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
readonly TRACK="${1:-core}"

readonly CORE_MIN_DISK_KIB=4194304
readonly CORE_MIN_MEMORY_KIB=2097152
readonly GROK_MIN_DISK_KIB=8388608
readonly GROK_MIN_MEMORY_KIB=6291456

case "${TRACK}" in
  core)
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
