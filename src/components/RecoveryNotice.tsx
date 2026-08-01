import { useEffect, useRef, useState } from "react";
import { AlertTriangle, X } from "lucide-react";
import type { RecoveryPresentation } from "../types";

export const RECOVERY_TITLE_ALERT_MS = 8_000;

interface RecoveryNoticeProps {
  projectName: string;
  recoveryPresentation: RecoveryPresentation | null;
}

export function RecoveryNotice({ projectName, recoveryPresentation }: RecoveryNoticeProps) {
  const projectScopeRef = useRef(projectName);
  const seenFingerprintsRef = useRef(new Set<string>());
  const [visibleFingerprint, setVisibleFingerprint] = useState<string | null>(null);
  const [attentionActive, setAttentionActive] = useState(false);
  const originalTitleRef = useRef("");
  const recovery = recoveryPresentation?.kind !== "None" ? recoveryPresentation : null;
  const fingerprint = recovery?.state_fingerprint ?? "";

  useEffect(() => {
    if (projectScopeRef.current !== projectName) {
      projectScopeRef.current = projectName;
      seenFingerprintsRef.current.clear();
      setVisibleFingerprint(null);
    }

    if (!recovery || !fingerprint) {
      setVisibleFingerprint(null);
      return;
    }

    const scopedFingerprint = `${projectName}:${fingerprint}`;
    if (seenFingerprintsRef.current.has(scopedFingerprint)) return;
    seenFingerprintsRef.current.add(scopedFingerprint);
    setVisibleFingerprint(fingerprint);
  }, [fingerprint, projectName, recovery]);

  useEffect(() => {
    if (!recovery || visibleFingerprint !== fingerprint) return;
    if (!document.hidden && document.hasFocus()) return;

    originalTitleRef.current = document.title;
    const alertTitle = `【需处理】${recovery.title}`;
    let highlighted = true;
    setAttentionActive(true);
    document.title = `${alertTitle} · ${originalTitleRef.current}`;
    const interval = window.setInterval(() => {
      highlighted = !highlighted;
      document.title = highlighted
        ? `${alertTitle} · ${originalTitleRef.current}`
        : originalTitleRef.current;
    }, 900);
    const stopAttention = () => {
      window.clearInterval(interval);
      document.title = originalTitleRef.current;
      setAttentionActive(false);
    };
    const timeout = window.setTimeout(stopAttention, RECOVERY_TITLE_ALERT_MS);
    window.addEventListener("focus", stopAttention, { once: true });
    document.addEventListener("visibilitychange", stopAttention, { once: true });

    if (typeof Notification !== "undefined" && Notification.permission === "granted") {
      try {
        new Notification(recovery.title, { body: recovery.reason, tag: fingerprint });
      } catch {
        // Window title and the in-app notice remain the dependency-free fallback.
      }
    }

    return () => {
      window.clearTimeout(timeout);
      window.clearInterval(interval);
      window.removeEventListener("focus", stopAttention);
      document.removeEventListener("visibilitychange", stopAttention);
      document.title = originalTitleRef.current;
      setAttentionActive(false);
    };
  }, [fingerprint, recovery, visibleFingerprint]);

  if (!recovery || visibleFingerprint !== fingerprint) return null;

  return (
    <div
      className={`recovery-notice recovery-notice-${recovery.severity.toLowerCase()}${attentionActive ? " recovery-notice-attention" : ""}`}
      data-recovery-notice-fingerprint={fingerprint}
      role="status"
      aria-live="assertive"
      aria-atomic="true"
    >
      <AlertTriangle size={18} aria-hidden="true" />
      <div className="recovery-notice-content">
        <strong>{recovery.title}</strong>
        <span>{recovery.reason}</span>
        <small>恢复操作已在下方控制条中就绪。</small>
      </div>
      <button
        type="button"
        className="recovery-notice-dismiss"
        onClick={() => setVisibleFingerprint(null)}
        aria-label="关闭本次恢复提醒"
      >
        <X size={15} />
      </button>
    </div>
  );
}
