// src/ProjectEntry.tsx — Before 页面：No Project / Half Project 双入口
import { useState } from "react";
import { invokeWithTimeout } from "./utils/invokeWithTimeout";
import { ProjectEntryKind, Project, PathValidationResult } from "./types";
import { Modal } from "./components/Modal";
import { Sparkles, FolderOpen, FolderPlus, ArrowRight } from "lucide-react";

interface ProjectEntryProps {
  onProjectCreated: (project: Project) => void;
}

const ENTRY_CARDS = [
  {
    kind: "NoProject" as ProjectEntryKind,
    icon: Sparkles,
    title: "从零开始",
    description: "创建一个全新的项目。Metheus 会帮你从讨论需求到生成代码，一步步搭建完整项目。",
  },
  {
    kind: "HalfProject" as ProjectEntryKind,
    icon: FolderOpen,
    title: "改造已有项目",
    description: "基于已有的代码项目进行改造。Metheus 会先分析现有代码结构，再制定改造方案。",
  },
];

export function ProjectEntry({ onProjectCreated }: ProjectEntryProps) {
  const [selectedKind, setSelectedKind] = useState<ProjectEntryKind | null>(null);
  const [projectName, setProjectName] = useState("");
  const [projectPath, setProjectPath] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [showCreateConfirm, setShowCreateConfirm] = useState(false);

  const handleSelectKind = (kind: ProjectEntryKind) => {
    setSelectedKind(kind);
    setError("");
    setProjectPath("");
  };

  const handleSubmit = async () => {
    setError("");
    if (!selectedKind) { setError("请先选择项目类型"); return; }
    if (!projectName.trim()) { setError("请输入项目名称"); return; }
    if (!projectPath.trim()) { setError("请输入项目路径"); return; }

    setLoading(true);
    try {
      // Validate path first
      const validation = await invokeWithTimeout<PathValidationResult>("validate_project_path", {
        projectPath: projectPath.trim(),
      });

      if (selectedKind === "NoProject") {
        // Path exists but is a regular file → reject immediately
        if (validation.exists && !validation.is_directory) {
          setError("路径已存在但是一个普通文件，不是目录。请选择一个目录路径。");
          setLoading(false);
          return;
        }
        // Path doesn't exist → show create confirm dialog
        if (!validation.exists) {
          setLoading(false);
          setShowCreateConfirm(true);
          return;
        }
        // Path exists and is directory → check if empty
        if (validation.is_directory) {
          const files = await invokeWithTimeout<any[]>("get_project_files", {
            projectPath: projectPath.trim(),
          });
          if (files.length > 0) {
            setError("该目录已包含文件。如要改造已有项目，请选择「改造已有项目」入口。");
            setLoading(false);
            return;
          }
        }
      }

      if (selectedKind === "HalfProject") {
        if (!validation.exists || !validation.is_directory) {
          setError("路径不存在或不是目录。");
          setLoading(false);
          return;
        }
        const files = await invokeWithTimeout<any[]>("get_project_files", {
          projectPath: projectPath.trim(),
        });
        if (files.length === 0) {
          setError("该目录为空。如要创建新项目，请选择「从零开始」入口。");
          setLoading(false);
          return;
        }
      }

      await createProject();
    } catch (e: any) {
      setError(String(e));
    } finally {
      if (!showCreateConfirm) setLoading(false);
    }
  };

  const handleConfirmCreate = async () => {
    setShowCreateConfirm(false);
    setLoading(true);
    try {
      await createProject();
    } catch (e: any) {
      // Keep user input visible on failure
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const createProject = async () => {
    const project = await invokeWithTimeout<Project>("initialize_project_entry", {
      projectName: projectName.trim(),
      projectPath: projectPath.trim(),
      entryKind: selectedKind,
    });
    onProjectCreated(project);
  };

  return (
    <div className="project-entry-container">
      <div className="project-entry-logo">
        <Sparkles size={40} />
      </div>
      <div className="project-entry-title">Metheus</div>
      <div className="project-entry-subtitle">
        复杂任务编译系统 — 把模糊想法编译为可执行、可检查、可回退的代码变更
      </div>

      <div className="project-entry-cards">
        {ENTRY_CARDS.map((card) => {
          const Icon = card.icon;
          return (
            <div
              key={card.kind}
              className={`project-entry-card ${selectedKind === card.kind ? "selected" : ""}`}
              onClick={() => handleSelectKind(card.kind)}
            >
              <div className="project-entry-card-icon">
                <Icon size={32} />
              </div>
              <div className="project-entry-card-title">{card.title}</div>
              <div className="project-entry-card-desc">{card.description}</div>
            </div>
          );
        })}
      </div>

      {selectedKind && (
        <div className="project-entry-form">
          <div className="project-entry-field">
            <label htmlFor="proj-name">项目名称</label>
            <input
              id="proj-name"
              type="text"
              value={projectName}
              onChange={(e) => setProjectName(e.target.value)}
              placeholder="例如：my-awesome-app"
              disabled={loading}
            />
          </div>
          <div className="project-entry-field">
            <label htmlFor="proj-path">
              {selectedKind === "NoProject" ? "目标路径" : "已有项目路径"}
            </label>
            <input
              id="proj-path"
              type="text"
              value={projectPath}
              onChange={(e) => { setProjectPath(e.target.value); setError(""); }}
              placeholder={
                selectedKind === "NoProject"
                  ? "例如：/home/user/my-project"
                  : "例如：/home/user/existing-project"
              }
              disabled={loading}
            />
          </div>

          {error && <div className="project-entry-error">{error}</div>}

          <button
            className="project-entry-submit"
            onClick={handleSubmit}
            disabled={loading || !projectName.trim() || !projectPath.trim()}
          >
            {loading ? "处理中..." : (
              <span style={{ display: "inline-flex", alignItems: "center", gap: "8px" }}>
                {selectedKind === "NoProject" ? "开始讨论" : "读取项目"}
                <ArrowRight size={16} />
              </span>
            )}
          </button>
        </div>
      )}

      <Modal
        isOpen={showCreateConfirm}
        onClose={() => setShowCreateConfirm(false)}
        title="确认创建目录"
        description={`路径「${projectPath}」尚不存在。确认后将自动创建该目录。`}
        actions={[
          { label: "取消", onClick: () => setShowCreateConfirm(false), variant: "secondary" },
          { label: "确认创建", onClick: handleConfirmCreate, variant: "primary" },
        ]}
      >
        <div style={{ display: "flex", alignItems: "center", gap: "12px", padding: "8px 0" }}>
          <FolderPlus size={24} style={{ flexShrink: 0 }} />
          <div>
            <div style={{ fontSize: "14px", fontWeight: 500 }}>{projectName}</div>
            <div style={{ fontSize: "12px", color: "#656d76" }}>{projectPath}</div>
          </div>
        </div>
      </Modal>
    </div>
  );
}
