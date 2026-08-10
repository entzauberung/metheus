// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
import React, { useState, useEffect, useCallback, useRef } from "react";
import { invokeWithTimeout } from "./utils/invokeWithTimeout";
import { FileEntry } from "./types";

interface TreeNode {
  name: string;
  path: string;
  isDir: boolean;
  fileType: string;
  children: TreeNode[];
  depth: number;
}

interface Props {
  projectPath: string;
  onFileSelect?: (path: string) => void;
}

type FileTreeStatus = "loading" | "error" | "empty" | "ready";

function describeFileTreeError(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message;
  if (typeof error === "string" && error.trim()) return error;
  return "无法读取项目文件，请重试";
}

/** 将平铺文件列表转换为树状结构 */
function buildTree(files: FileEntry[], projectPath: string): TreeNode[] {
  // 提取项目根目录名
  const rootName = projectPath.split("/").filter(Boolean).pop() || "project";
  const root: TreeNode = {
    name: rootName,
    path: projectPath,
    isDir: true,
    fileType: "",
    children: [],
    depth: 0,
  };

  // 按 path 排序：目录优先，然后按名称字母序
  const sorted = [...files].sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    return a.path.localeCompare(b.path);
  });

  for (const entry of sorted) {
    const parts = entry.path.split("/");
    let current = root;

    // 逐级插入
    for (let i = 0; i < parts.length; i++) {
      const isLast = i === parts.length - 1;
      const partName = parts[i];
      const partialPath = parts.slice(0, i + 1).join("/");

      if (isLast) {
        // 叶子节点
        current.children.push({
          name: partName,
          path: entry.path,
          isDir: entry.is_dir,
          fileType: entry.file_type,
          children: [],
          depth: i + 1,
        });
      } else {
        // 中间目录节点
        let child = current.children.find(
          (c) => c.name === partName && c.isDir
        );
        if (!child) {
          child = {
            name: partName,
            path: partialPath,
            isDir: true,
            fileType: "",
            children: [],
            depth: i + 1,
          };
          current.children.push(child);
        }
        current = child;
      }
    }
  }

  return [root];
}

/** 根据文件类型返回图标 */
function getFileIcon(fileType: string, isDir: boolean): string {
  if (isDir) return "\u{1F4C1}"; // 📁
  switch (fileType) {
    case "tsx":
    case "jsx":
      return "⚛️"; // ⚛️
    case "ts":
      return "\u{1F537}"; // 🔷
    case "js":
      return "\u{1F7E8}"; // 🟨
    case "rs":
      return "\u{1F980}"; // 🦀
    case "py":
      return "\u{1F40D}"; // 🐍
    case "go":
      return "\u{1F535}"; // 🔵
    case "json":
      return "\u{1F4CB}"; // 📋
    case "md":
      return "\u{1F4DD}"; // 📝
    case "css":
    case "scss":
    case "less":
      return "\u{1F3A8}"; // 🎨
    case "html":
      return "\u{1F310}"; // 🌐
    case "yaml":
    case "yml":
      return "⚙️"; // ⚙️
    case "toml":
      return "\u{1F4E6}"; // 📦
    case "gitignore":
    case "env":
      return "\u{1F527}"; // 🔧
    case "svg":
    case "png":
    case "jpg":
    case "jpeg":
    case "gif":
      return "\u{1F5BC}️"; // 🖼️
    default:
      return "\u{1F4C4}"; // 📄
  }
}

function FileTree({ projectPath, onFileSelect }: Props) {
  const [tree, setTree] = useState<TreeNode[]>([]);
  const [status, setStatus] = useState<FileTreeStatus>(projectPath ? "loading" : "empty");
  const [errorMessage, setErrorMessage] = useState("");
  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set());
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const requestIdRef = useRef(0);

  const fetchFiles = useCallback(async () => {
    const requestId = ++requestIdRef.current;
    setTree([]);
    setErrorMessage("");
    setSelectedPath(null);
    if (!projectPath) {
      setStatus("empty");
      setExpandedDirs(new Set());
      return;
    }

    setStatus("loading");
    try {
      const result = await invokeWithTimeout<FileEntry[]>("get_project_files", {
        projectPath,
      });
      if (requestId !== requestIdRef.current) return;
      if (result.length === 0) {
        setStatus("empty");
        setExpandedDirs(new Set());
        return;
      }
      setTree(buildTree(result, projectPath));
      setExpandedDirs(new Set([projectPath]));
      setStatus("ready");
    } catch (error) {
      if (requestId !== requestIdRef.current) return;
      console.error("获取文件列表失败:", error);
      setTree([]);
      setExpandedDirs(new Set());
      setErrorMessage(describeFileTreeError(error));
      setStatus("error");
    }
  }, [projectPath]);

  // 获取文件列表；失效的旧请求不能覆盖新项目的状态。
  useEffect(() => {
    void fetchFiles();
    return () => {
      requestIdRef.current += 1;
    };
  }, [fetchFiles]);

  const toggleDir = useCallback((path: string) => {
    setExpandedDirs((prev) => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  }, []);

  const handleNodeClick = useCallback(
    (node: TreeNode) => {
      if (node.isDir) {
        toggleDir(node.path);
      } else {
        setSelectedPath(node.path);
        onFileSelect?.(node.path);
      }
    },
    [toggleDir, onFileSelect]
  );

  // 渲染单个树节点（递归）
  const renderNode = useCallback(
    (node: TreeNode): React.ReactNode => {
      const paddingLeft = node.depth * 16 + 8;
      const isSelected = selectedPath === node.path;
      const isDirExpanded = expandedDirs.has(node.path);

      return (
        <div key={node.path}>
          <button
            className={`tree-node ${node.isDir ? "dir-node" : "file-node"}${isSelected ? " selected" : ""}`}
            style={{ paddingLeft: `${paddingLeft}px` }}
            onClick={() => handleNodeClick(node)}
            type="button"
          >
            <span className="tree-node-icon">
              {node.isDir ? (isDirExpanded ? "\u{1F4C2}" : "\u{1F4C1}") : getFileIcon(node.fileType, false)}
            </span>
            <span className="tree-node-name" title={node.name}>
              {node.name}
            </span>
          </button>
          {node.isDir && isDirExpanded && node.children.length > 0 && (
            <div className="tree-children">
              {node.children.map((child) => renderNode(child))}
            </div>
          )}
        </div>
      );
    },
    [selectedPath, expandedDirs, handleNodeClick]
  );

  return (
    <div className="file-tree-container file-tree" aria-label="项目文件">
      <div className="file-tree-content">
        <div className="file-tree-header">
          <div className="file-tree-heading">
            <span className="file-tree-title">📂 项目文件</span>
            <span className="file-tree-path" title={projectPath || "未选择项目"}>
              {projectPath || "未选择项目"}
            </span>
          </div>
          <button
            className="file-tree-refresh"
            type="button"
            disabled={status === "loading" || !projectPath}
            onClick={() => { void fetchFiles(); }}
            title={status === "error" ? "重试读取文件列表" : "刷新文件列表"}
          >
            {status === "error" ? "重试" : "刷新"}
          </button>
        </div>

        {status === "loading" && (
          <div className="file-tree-state file-tree-loading" role="status" aria-live="polite">
            正在读取项目文件…
          </div>
        )}
        {status === "error" && (
          <div className="file-tree-state file-tree-error" role="alert" aria-live="assertive">
            <strong>文件列表读取失败</strong>
            <span>{errorMessage}</span>
            <button type="button" onClick={() => { void fetchFiles(); }}>重试</button>
          </div>
        )}
        {status === "empty" && (
          <div className="file-tree-state file-tree-empty" role="status" aria-live="polite">
            {projectPath ? "当前项目为空目录" : "请先选择项目目录"}
          </div>
        )}
        {status === "ready" && (
          <div className="file-tree-scroll" aria-label="项目文件列表">
            {tree.map((node) => renderNode(node))}
          </div>
        )}
      </div>
    </div>
  );
}

export default FileTree;
