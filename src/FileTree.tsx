// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
import React, { useState, useEffect, useCallback } from "react";
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
  const [pinned, setPinned] = useState(false);
  const [isHovered, setIsHovered] = useState(false);
  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set());
  const [selectedPath, setSelectedPath] = useState<string | null>(null);

  // 获取文件列表
  useEffect(() => {
    if (!projectPath) return;
    const fetchFiles = async () => {
      try {
        const result = await invokeWithTimeout<FileEntry[]>("get_project_files", {
          projectPath,
        });
        const treeData = buildTree(result, projectPath);
        setTree(treeData);
        // 默认展开根目录
        setExpandedDirs(new Set([projectPath]));
      } catch (e) {
        console.error("获取文件列表失败:", e);
        setTree([]);
      }
    };
    fetchFiles();
  }, [projectPath]);

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

  const isExpanded = pinned || isHovered;
  const panelWidth = isExpanded ? 220 : 4;

  // 渲染单个树节点（递归）
  const renderNode = useCallback(
    (node: TreeNode): React.ReactNode => {
      const paddingLeft = node.depth * 16 + 8;
      const isSelected = selectedPath === node.path;
      const isDirExpanded = expandedDirs.has(node.path);

      return (
        <div key={node.path}>
          <div
            className={`tree-node ${node.isDir ? "dir-node" : "file-node"}${isSelected ? " selected" : ""}`}
            style={{ paddingLeft: `${paddingLeft}px` }}
            onClick={() => handleNodeClick(node)}
          >
            <span className="tree-node-icon">
              {node.isDir ? (isDirExpanded ? "\u{1F4C2}" : "\u{1F4C1}") : getFileIcon(node.fileType, false)}
            </span>
            <span className="tree-node-name" title={node.name}>
              {node.name}
            </span>
          </div>
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

  if (!projectPath) {
    return (
      <div className="file-tree-container" style={{ width: 4 }}>
        <div className="file-tree-indicator" />
      </div>
    );
  }

  return (
    <div
      className={`file-tree-container${pinned ? " pinned" : ""}`}
      style={{ width: panelWidth }}
      onMouseEnter={() => setIsHovered(true)}
      onMouseLeave={() => setIsHovered(false)}
    >
      {/* 触发指示条 */}
      <div
        className="file-tree-indicator"
        onClick={() => setPinned(!pinned)}
        title={pinned ? "点击取消固定" : "点击固定文件树"}
      />

      {/* 文件树内容 */}
      {isExpanded && (
        <div className="file-tree-content">
          <div className="file-tree-header">
            <span className="file-tree-title">📂 项目文件</span>
            <button
              className="file-tree-refresh"
              onClick={async () => {
                try {
                  const result = await invokeWithTimeout<FileEntry[]>(
                    "get_project_files",
                    { projectPath }
                  );
                  setTree(buildTree(result, projectPath));
                } catch (_) {
                  /* ignore */
                }
              }}
              title="刷新文件列表"
            >
              🔄
            </button>
          </div>
          {tree.length === 0 ? (
            <div className="file-tree-empty">当前项目为空目录</div>
          ) : (
            <div className="file-tree-scroll">
              {tree.map((node) => renderNode(node))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export default FileTree;
