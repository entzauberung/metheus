// src/components/DecisionStepHeader.tsx — 决策层步骤导航
import { WorkflowStep } from "../types";
import { MessageCircle, ClipboardCheck, FileCheck, Rocket } from "lucide-react";

interface DecisionStepHeaderProps {
  currentStep: WorkflowStep;
}

const steps: { step: WorkflowStep; label: string; icon: React.ReactNode; desc: string }[] = [
  { step: "Discussion", label: "需求讨论", icon: <MessageCircle size={16} />, desc: "与 AI 讨论项目目标" },
  { step: "ThreeChecks", label: "三项审查", icon: <ClipboardCheck size={16} />, desc: "通过三项质量检查" },
  { step: "PlanApproval", label: "方案审批", icon: <FileCheck size={16} />, desc: "审核并批准项目方案" },
  { step: "MilestoneGeneration", label: "进入控制台", icon: <Rocket size={16} />, desc: "开始规划执行" },
];

export function DecisionStepHeader({ currentStep }: DecisionStepHeaderProps) {
  const currentIdx = steps.findIndex(s => s.step === currentStep);

  return (
    <div style={{
      display: "flex", alignItems: "center", justifyContent: "center",
      padding: "12px 16px", borderBottom: "1px solid #d0d7de",
      background: "#f6f8fa", gap: "0",
    }}>
      {steps.map((s, i) => {
        const isActive = s.step === currentStep;
        const isPast = i < currentIdx;
        return (
          <div key={s.step} style={{
            display: "flex", alignItems: "center", gap: "0",
            opacity: isPast || isActive ? 1 : 0.45,
          }}>
            {i > 0 && (
              <div style={{
                width: "24px", height: "2px",
                background: isPast ? "#1a7f37" : "#d0d7de",
                margin: "0 8px",
              }} />
            )}
            <div style={{
              display: "flex", alignItems: "center", gap: "5px",
              padding: "4px 10px", borderRadius: "20px",
              background: isActive ? "#dafbe1" : "transparent",
              border: isActive ? "1px solid #1a7f37" : "1px solid transparent",
              fontWeight: isActive ? 600 : 400,
              fontSize: "13px", color: isPast ? "#1a7f37" : isActive ? "#1a7f37" : "#656d76",
            }}
              title={s.desc}
            >
              {isPast ? <span style={{ fontSize: "14px" }}>✅</span> : s.icon}
              <span className="step-label" style={{ whiteSpace: "nowrap" }}>{s.label}</span>
            </div>
          </div>
        );
      })}
    </div>
  );
}
