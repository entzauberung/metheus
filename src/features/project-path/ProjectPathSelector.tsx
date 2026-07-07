import { useCallback, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { invokeWithTimeout } from "../../utils/invokeWithTimeout";
import { PathValidationResult, Project } from "../../types";
import "./projectPath.css";

interface ProjectPathSelectorProps {
  project: Project | null;
  projectPath: string;
  onProjectChange: (project: Project) => void;
  onProjectPathChange: (projectPath: string) => void;
}

function ProjectPathSelector({
  project,
  projectPath,
  onProjectChange,
  onProjectPathChange,
}: ProjectPathSelectorProps) {
  const [pathSaveStatus, setPathSaveStatus] = useState("");

  const persistProjectPath = useCallback(async (nextProjectPath: string) => {
    if (!project) return;

    const updated = { ...project, project_path: nextProjectPath };
    let nextStatus = "✅ 已保存";

    // 验证路径（仅提示，不阻止保存）
    if (nextProjectPath) {
      try {
        const validation = await invokeWithTimeout<PathValidationResult>("validate_project_path", {
          projectPath: nextProjectPath,
        });
        if (!validation.is_valid) {
          nextStatus = `⚠️ ${validation.error_message}（已保存）`;
        }
      } catch (_) { /* 验证失败不影响保存 */ }
    }

    try {
      await invokeWithTimeout("persist_project", { projectJson: JSON.stringify(updated) });
      onProjectChange(updated);
      onProjectPathChange(nextProjectPath);
      setPathSaveStatus(nextStatus);
      setTimeout(() => setPathSaveStatus(""), 5000);
    } catch (e: any) {
      setPathSaveStatus(`❌ 保存失败：${e}`);
    }
  }, [onProjectChange, onProjectPathChange, project]);

  const handleSelectProjectPath = useCallback(async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "选择项目目录",
        defaultPath: projectPath || undefined,
        canCreateDirectories: true,
      });

      if (typeof selected !== "string") return;
      await persistProjectPath(selected);
    } catch (e: any) {
      setPathSaveStatus(`❌ 选择目录失败：${e}`);
    }
  }, [persistProjectPath, projectPath]);

  return (
    <div className="project-path-section">
      <h3>📁 项目目录</h3>
      <div className="project-path-row">
        <input
          className="project-path-input"
          type="text"
          value={projectPath}
          readOnly
          onClick={handleSelectProjectPath}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              handleSelectProjectPath();
            }
          }}
          placeholder="点击选择项目目录"
          title={projectPath || "点击选择项目目录"}
        />
        <button
          type="button"
          className="btn-select-path"
          onClick={handleSelectProjectPath}
        >
          选择
        </button>
      </div>
      {pathSaveStatus && (
        <div className={`path-status ${pathSaveStatus.startsWith("✅") ? "success" : "error"}`}>
          {pathSaveStatus}
        </div>
      )}
    </div>
  );
}

export default ProjectPathSelector;
