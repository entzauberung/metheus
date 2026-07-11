import { useState } from "react";
import { invokeWithTimeout } from "../../utils/invokeWithTimeout";
import "./executionEngineTestPanel.css";

type TestCommand = "" | "execute" | "check" | "prompt";

interface ExecutionEngineTestPanelProps {
  projectPath: string;
}

function ExecutionEngineTestPanel({ projectPath }: ExecutionEngineTestPanelProps) {
  const [testResult, setTestResult] = useState("");
  const [testLoading, setTestLoading] = useState<TestCommand>("");

  return (
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
              const res: any = await invokeWithTimeout("execute_subtask", {
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
              const res: any = await invokeWithTimeout("check_subtask", {
                projectPath: projectPath || "/tmp/test",
                subtaskId: "st-test-001",
                subtaskGoal: "创建测试文件",
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
          {testLoading === "check" ? "⏳ 检查中..." : "🔍 check_subtask"}
        </button>
        <button
          className="test-btn"
          disabled={testLoading === "prompt"}
          onClick={async () => {
            setTestLoading("prompt");
            setTestResult("");
            try {
              const res: any = await invokeWithTimeout("generate_next_prompt", {
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
  );
}

export default ExecutionEngineTestPanel;
