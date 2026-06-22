// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";
import { Project } from "./types";
import ExecutionTree from "./ExecutionTree";
import ChatRoom from "./ChatRoom";

function App() {
  const [project, setProject] = useState<Project | null>(null);
  const [projectPath, setProjectPath] = useState<string>("");
  const [pathSaveStatus, setPathSaveStatus] = useState<string>("");
  const [testResult, setTestResult] = useState<string>("");
  const [testLoading, setTestLoading] = useState<string>("");

  // 完整回调 当 App 组件第一次加载时，自动从后端（Rust）获取当前项目数据，并保存到前端的状态中
  // 页面一打开，自动从后端拉取项目数据，存到状态里，并保存项目路径，只做一次
  useEffect(() => {
    invoke<Project>("get_project", { projectName: "我的游戏" })
      .then((project) => {
        setProject(project);
        if (project && project.project_path) {
          setProjectPath(project.project_path);
        }
      })
      .catch((err) => {
        console.error("获取项目失败:", err);
        setProject(null);
      });
  }, []);

  const handleAddMessage = (msg: any) => {
    if (!project) return;
    const updatedProject = { ...project };
    if (updatedProject.discussion_threads.length > 0) {
      updatedProject.discussion_threads[0].messages = [
        ...updatedProject.discussion_threads[0].messages,
        msg,
      ];
    }
    setProject(updatedProject);
  };

  const handleSelectMilestone = (id: string) => {
    console.log("selected:", id);
  };
  //生成版本方案
  const handleGeneratePlan = async () => {
    //无项目数据则返回
    if (!project) return;
    const currentThread = project.discussion_threads[0];
    if (!currentThread) {
      console.error("没有可用的讨论线程");
      return;
    }
    try {
      const plan = await invoke("generate_version_plan", {
        messages: currentThread.messages,
        projectPath: project.project_path,
      });
      //更新项目，加上方案
      setProject({ ...project, version_plan: plan as string });
    } catch (err) {
      console.error("生成方案失败", err);
    }
  };
  //批准版本方案
  const handleApprove = async () => {
    //安全保护
    if (!project) return;
    try {
      await invoke("approve_version_plan", {
        //把项目转成 JSON 字符串传到后端
        projectJson: JSON.stringify(project),
        //再单独传一次方案（其实后端可以从projectJson里取
        versionPlan: project.version_plan,
      });
      //前端也同步状态"规划中"
      setProject({ ...project, status: "Planning" });
    } catch (err) {
      console.error("批准失败：", err);
    }
  };
  //驳回版本方案（不调用后端，硬盘存储还在，临时放弃）
  const handleReject = () => {
    if (!project) return;
    //直接把方案清空
    setProject({ ...project, version_plan: "", status: "Discussing" });
  }

  //根据版本方案拆解大阶段
  const handleGenerateMilestones = async () => {
    if (!project) return;
    //调用后端命令，传入版本方案和模式，等待返回Milestones数组
    try {
      const milestones = await invoke("generate_milestones", {
        versionPlan: project.version_plan,
        mode: project.mode,
      });
      //把Milestones数组合并到项目状态中（触发重新渲染）
      setProject({ ...project, status: "MilestoneReady", milestones: milestones as any[] });
    } catch (err) {
      console.error("拆解大阶段失败：", err);
    }
  };

  //切换项目的工作模式（快速/专业）
  const handleModeChange = (mode: "Quick" | "Professional") => {
    if (!project) return;
    const updatedProject = { ...project, mode };
    setProject(updatedProject);
    //保存到文件
    invoke("persist_project", { projectJson: JSON.stringify(updatedProject) }).catch(console.error);
  };

  // 编辑大阶段版本号
  const handleVersionEdit = (id: string, newVersion: string) => {
    if (!project) return;
    const updatedMilestones = project.milestones.map(ms =>
      ms.id === id ? { ...ms, version: newVersion } : ms
    );
    const updatedProject = { ...project, milestones: updatedMilestones };
    setProject(updatedProject);
  };

  //点拆解中阶段，调后端拆解子阶段，更新到项目状态中
  const handleGenerateMidStages = async (milestoneId: string) => {
    if (!project) return;
    const milestone = project.milestones.find(ms => ms.id === milestoneId);
    if (!milestone) return;
    try {
      const midStages = await invoke("generate_mid_stages", {
        milestoneId: milestoneId,
        milestoneTitle: milestone.title,
        milestoneDescription: milestone.description,
        versionPlan: project.version_plan,
        mode: project.mode,
        attentionPoints: milestone.qa_result?.attention_points ?? [],
      });
      const updatedMilestones = project.milestones.map(ms =>
        ms.id === milestoneId ? { ...ms, mid_stages: midStages as any[] } : ms
      );
      setProject({ ...project, milestones: updatedMilestones });
    } catch (err) {
      console.error("拆解中阶段失败：", err);
    }
  }

  //当质检驳回后，用户选择「采纳意见，重新拆解」时调用
  const handleRegenerateMilestones = async (feedback: string) => {
    if (!project) return;
    try {
      const newMilestones = await invoke("regenerate_milestones_with_feedback", {
        versionPlan: project.version_plan,
        mode: project.mode,
        feedback: feedback,
      });
      setProject({ ...project, status: "MilestoneReady", milestones: newMilestones as any[] });
    } catch (err) {
      console.error("重新拆解失败：", err);
      alert("重新拆解失败：" + err);
    }
  };

  //根据项目状态返回默认对话角色
  const getDefaultRole = (status: string): string => {
    switch (status) {
      case "Idle":
      case "Discussing":
        return "策略产品经理";
      case "Planning":
        return "产品经理";
      case "MilestoneReady":
        return "域负责人";
      case "Executing":
        return "全栈技术顾问";
      case "Paused":
        return "策略产品经理"
      default:
        return "策略产品经理";
    }
  };

  if (!project) {
    return <div className="loading">加载中...</div>;
  }

  const currentThread = project.discussion_threads[0];
  if (!currentThread) {
    return <div className="loading">没有可用的讨论线程，请检查项目数据...</div>;
  }

  return (
    <div className="app-layout">
      <aside className="sidebar">
        <ExecutionTree
          milestones={project.milestones}
          onSelectMilestone={handleSelectMilestone}
          onVersionEdit={handleVersionEdit}
          onGenerateMidStages={handleGenerateMidStages}
          onRegenerateMilestones={handleRegenerateMilestones}
          projectPath={projectPath}
          projectId={project.name}
        />
      </aside>

      <main className="main-content">
        {/* === 开发者工具：让你可以手动测试 Tauri 后端的“执行子任务”、“
        检查子任务”、“生成下一步提示词”三个核心命令，并查看结果 === */}
        <div className="project-path-section">
          <h3>📁 项目目录</h3>
          <div className="project-path-row">
            <input
              className="project-path-input"
              type="text"
              value={projectPath}
              onChange={(e) => setProjectPath(e.target.value)}
              placeholder="例如：/home/user/my-project"
            />
            <button
              className="btn-save-path"
              onClick={async () => {
                const updated = { ...project, project_path: projectPath };
                try {
                  await invoke("persist_project", { projectJson: JSON.stringify(updated) });
                  setProject(updated);
                  setPathSaveStatus("✅ 已保存");
                  setTimeout(() => setPathSaveStatus(""), 3000);
                } catch (e: any) {
                  setPathSaveStatus(`❌ 保存失败：${e}`);
                }
              }}
            >
              保存
            </button>
          </div>
          {pathSaveStatus && (
            <div className={`path-status ${pathSaveStatus.startsWith("✅") ? "success" : "error"}`}>
              {pathSaveStatus}
            </div>
          )}
        </div>
        {/* === Phase 2：命令测试面板 === */}
        <div className="test-panel">
          <h3>🔧 执行引擎测试</h3>
          <div className="test-buttons">
            <button
              className="test-btn"
              disabled={testLoading === "execute"}
              onClick={async () => {
                setTestLoading("execute");
                setTestResult("");
                try {
                  const res: any = await invoke("execute_subtask", {
                    projectPath: projectPath || "/tmp/test",
                    prompt: "创建一个 metheus_test.txt 文件",
                    subtaskId: "st-test-001",
                    milestoneId: "ms-test-001",
                    midStageId: "mid-test-001",
                  });
                  setTestResult(JSON.stringify(res, null, 2));
                } catch (e: any) {
                  setTestResult(`❌ 错误：${e}`);
                } finally {
                  setTestLoading("");
                }
              }}
            >
              {testLoading === "execute" ? "⏳ 执行中..." : "▶ execute_subtask"}
            </button>
            <button
              className="test-btn"
              disabled={testLoading === "check"}
              onClick={async () => {
                setTestLoading("check");
                setTestResult("");
                try {
                  const res: any = await invoke("check_subtask", {
                    projectPath: projectPath || "/tmp/test",
                    subtaskId: "st-test-001",
                    subtaskGoal: "创建测试文件",
                  });
                  setTestResult(JSON.stringify(res, null, 2));
                } catch (e: any) {
                  setTestResult(`❌ 错误：${e}`);
                } finally {
                  setTestLoading("");
                }
              }}
            >
              {testLoading === "check" ? "⏳ 检查中..." : "🔍 check_subtask"}
            </button>
            <button
              className="test-btn"
              disabled={testLoading === "prompt"}
              onClick={async () => {
                setTestLoading("prompt");
                setTestResult("");
                try {
                  const res: any = await invoke("generate_next_prompt", {
                    midStageTitle: "数据库设计",
                    midStageDescription: "设计用户模型",
                    previousSubtaskTitle: "创建连接配置",
                    previousSubtaskResult: "通过",
                    fileChanges: ["config.ts"],
                    testResult: "通过",
                    isRetry: false,
                    retryReason: "",
                  });
                  setTestResult(JSON.stringify(res, null, 2));
                } catch (e: any) {
                  setTestResult(`❌ 错误：${e}`);
                } finally {
                  setTestLoading("");
                }
              }}
            >
              {testLoading === "prompt" ? "⏳ 生成中..." : "🤖 generate_next_prompt"}
            </button>
          </div>
          <div className={`test-result-box ${!testResult ? "empty" : ""}`}>
            {testResult || "点击上方按钮测试执行引擎命令，结果将显示在此处。"}
          </div>
        </div>

        <header className="chat-header">
          <h2>弥 · 工作流指挥中心</h2>
          <h4>Metheus 带来你的灵感，输出你的创意！</h4>
        </header>
        {(project.status === "Idle" || project.status === "Discussing") && !project.version_plan && (
          <div className="generate-plan-area">
            <button className="btn-generate-plan" onClick={handleGeneratePlan}>
              📝 生成版本方案
            </button>
          </div>
        )}
        <ChatRoom
          messages={currentThread.messages || []}
          onAddMessage={handleAddMessage}
          currentRole={getDefaultRole(project.status)}
          mode={project.mode}
          onModeChange={handleModeChange}
          modeLocked={project.status !== "Idle"}
        />
        {/* 版本方案区域：只在 Disscuss 状态且已生成时显示 */}
        {project.version_plan && (
          <div className="version-plan-panel">
            <h3>📋 版本方案摘要</h3>
            <pre className="version-plan-content">{project.version_plan}</pre>
            <div className="version-plan-actions">
              <button className="btn-approve" onClick={handleApprove}>✅ 批准</button>
              <button className="btn-reject" onClick={handleReject}>❌ 驳回</button>
            </div>
          </div>
        )}
        {/* 版本方案已批准，拆解大阶段 */}
        {project.status === "Planning" && project.version_plan && (
          <div className="generate-plan-area">
            <button className="btn-generate-plan" onClick={handleGenerateMilestones}>
              📊 根据版本方案拆解大阶段
            </button>
          </div>
        )}
      </main>
    </div>
  );
}

export default App;
