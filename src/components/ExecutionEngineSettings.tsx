import { useCallback, useEffect, useState } from "react";
import { Settings } from "lucide-react";
import { EngineHealth, ExecutionProfile, PipelineState, Project } from "../types";
import { invokeWithTimeout } from "../utils/invokeWithTimeout";
import { engineChangeBlockedReason, engineHealthBlocksExecution } from "../enginePolicy";
import { IconButton } from "./IconButton";
import { Modal } from "./Modal";
import { ExecutionEngineSelector, EngineHealthCheckState } from "./ExecutionEngineSelector";

interface Props {
  project: Project;
  pipeline: PipelineState | null;
  onProjectUpdated: (project: Project) => void;
}

export function ExecutionEngineSettings({ project, pipeline, onProjectUpdated }: Props) {
  const [open, setOpen] = useState(false);
  const [profile, setProfile] = useState<ExecutionProfile>(project.execution_profile);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [engineHealth, setEngineHealth] = useState<EngineHealth | null>(null);
  const [checkingEngine, setCheckingEngine] = useState(true);
  const blockedReason = engineChangeBlockedReason(project, pipeline);
  const changed = profile.runtime !== project.execution_profile.runtime
    || profile.provider !== project.execution_profile.provider
    || profile.permission_profile !== project.execution_profile.permission_profile;
  const engineUnavailable = checkingEngine || engineHealth === null
    || engineHealthBlocksExecution(engineHealth);
  const handleEngineHealthChange = useCallback(({ health, checking }: EngineHealthCheckState) => {
    setEngineHealth(health);
    setCheckingEngine(checking);
  }, []);
  const providerLabel = project.execution_profile.runtime === "BuiltIn"
    ? "Grok Build（内置）"
    : ({
    ClaudeCode: "Claude Code",
    Codex: "Codex",
    GrokBuild: "Grok Build CLI（本机）",
    KimiCli: "Kimi CLI",
  }[project.execution_profile.provider]);

  useEffect(() => {
    if (!open) setProfile(project.execution_profile);
  }, [open, project.execution_profile]);

  const handleOpen = () => {
    setProfile(project.execution_profile);
    setError("");
    setOpen(true);
  };

  const handleSave = async () => {
    if (!changed || blockedReason || engineUnavailable) return;
    setSaving(true);
    setError("");
    try {
      const updated = await invokeWithTimeout<Project>("update_execution_profile", {
        projectName: project.name,
        expectedDataRevision: project.workflow_state.data_revision,
        executionProfile: profile,
      });
      onProjectUpdated(updated);
      setOpen(false);
    } catch (saveError) {
      setError(String(saveError));
    } finally {
      setSaving(false);
    }
  };

  return (
    <>
      <span className="project-engine-name">
        {providerLabel}
      </span>
      <IconButton
        icon={<Settings size={16} />}
        tooltip="执行引擎设置"
        size="sm"
        onClick={handleOpen}
      />
      <Modal
        isOpen={open}
        onClose={() => setOpen(false)}
        title="执行引擎设置"
        lockClose={saving}
        isSubmitting={saving}
        actions={[
          { label: "取消", onClick: () => setOpen(false), variant: "secondary", disabled: saving },
          { label: "保存", onClick: handleSave, variant: "primary", disabled: !changed || Boolean(blockedReason) || engineUnavailable },
        ]}
      >
        <ExecutionEngineSelector
          value={profile}
          onChange={setProfile}
          disabled={Boolean(blockedReason) || saving}
          onHealthChange={handleEngineHealthChange}
        />
        {blockedReason && <div className="engine-settings-notice">{blockedReason}，暂停并结束当前会话后才能切换。</div>}
        {error && <div className="project-entry-error">{error}</div>}
      </Modal>
    </>
  );
}
