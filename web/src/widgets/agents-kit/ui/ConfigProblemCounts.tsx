import type { Problem } from "@/entities/agent-config";
import { Icon } from "@/shared/ui";

export function ConfigProblemCounts({ problems }: { problems: Problem[] }) {
  const errors = problems.filter((p) => p.level === "error");
  const warnings = problems.filter((p) => p.level === "warning");
  if (!problems.length) return null;
  return <span className="config-problem-counts">
    {errors.length > 0 && <span className="config-problem-count error" role="img" aria-label={`Ошибок: ${errors.length}`} title={errors.map((p) => p.message).join("\n")}>{errors.length}</span>}
    {warnings.length > 0 && <span className="config-problem-count warning" role="img" aria-label={`Предупреждений: ${warnings.length}`} title={warnings.map((p) => p.message).join("\n")}><Icon.alert size={12} />{warnings.length}</span>}
  </span>;
}
