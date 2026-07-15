import { CheckCircle2 } from "lucide-react";

interface CandidateField {
  label: string;
  value?: string | string[];
}

interface StageCandidateCardProps {
  title: string;
  version?: string;
  description?: string;
  fields?: CandidateField[];
  selected?: boolean;
  readOnly?: boolean;
  onSelect?: () => void;
}

export function StageCandidateCard({
  title,
  version,
  description,
  fields = [],
  selected = false,
  readOnly = true,
  onSelect,
}: StageCandidateCardProps) {
  const content = (
    <>
      <div className="candidate-card-heading">
        <strong>{title}</strong>
        {version && <span>{version}</span>}
        {selected && <CheckCircle2 size={16} aria-label="已选择" />}
      </div>
      {description && <p className="candidate-card-description">{description}</p>}
      {fields.map(({ label, value }) => {
        const text = Array.isArray(value) ? value.join("；") : value;
        if (!text) return null;
        return <div className="candidate-card-field" key={label}><dt>{label}</dt><dd>{text}</dd></div>;
      })}
    </>
  );

  if (!readOnly && onSelect) {
    return <button className={`stage-candidate-card selectable${selected ? " selected" : ""}`} onClick={onSelect}>{content}</button>;
  }
  return <article className={`stage-candidate-card${selected ? " selected" : ""}`}>{content}</article>;
}
