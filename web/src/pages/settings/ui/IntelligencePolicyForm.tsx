import { useState } from "react";
import type { IntelligencePolicy } from "@/shared/api/generated/IntelligencePolicy.ts";

const split = (text: string) => text.split(",").map((v) => v.trim()).filter(Boolean);
export function IntelligencePolicyForm({ value: p, onChange }: { value: IntelligencePolicy; onChange: (value: IntelligencePolicy) => void }) {
  const field = <K extends keyof IntelligencePolicy>(key: K, value: IntelligencePolicy[K]) => onChange({ ...p, [key]: value });
  return <section className="st-card" style={{ marginTop: "var(--g-space-4)" }}>
    <div className="st-card-head"><h3>Оценка задач и обязательные проверки</h3><p>Правила проекта действуют до выбора Jev. Неизвестная область изменения сохраняет проверки всех защищённых компонентов.</p></div>
    <div className="st-row"><label htmlFor="intelligence-enabled" className="lbl"><b>Применять политику оценки задач</b></label><div className="ctl"><input id="intelligence-enabled" type="checkbox" checked={p.enabled} onChange={(e) => field("enabled", e.target.checked)} /></div></div>
    <div className="st-row"><label htmlFor="intelligence-version" className="lbl"><b>Версия политики</b></label><div className="ctl"><input id="intelligence-version" className="st-input" value={p.version} required maxLength={120} onChange={(e) => field("version", e.target.value)} /></div></div>
    <div className="st-row"><label htmlFor="intelligence-semantic" className="lbl"><b>Оценивать сигналы через Jev</b><span>Дополнительный ограниченный вызов. Требуется разрешение на передачу контекста.</span></label><div className="ctl"><input id="intelligence-semantic" type="checkbox" checked={p.semanticAssessment} onChange={(e) => field("semanticAssessment", e.target.checked)} /></div></div>
    <div className="st-row"><label htmlFor="intelligence-fallback" className="lbl"><b>Безопасный резервный шаблон</b><span>Должен включать все обязательные проверки.</span></label><div className="ctl"><input id="intelligence-fallback" className="st-input" value={p.fallbackTemplate ?? ""} onChange={(e) => field("fallbackTemplate", e.target.value || null)} /></div></div>
    <div className="st-row"><label htmlFor="intelligence-cooldown" className="lbl"><b>Интервал переоценки, секунд</b><span>Обязательное усиление защиты допускается сразу.</span></label><div className="ctl"><input id="intelligence-cooldown" className="st-input" type="number" min={0} max={3600} value={p.cooldownSecs} onChange={(e) => field("cooldownSecs", Number(e.target.value))} /></div></div>
    <h4>Независимые проверки</h4>
    {Object.entries(p.checks).map(([check, role]) => <div key={check} className="st-row"><label className="lbl" htmlFor={`check-${check}`}><b>{check}</b><span>Роль независимого reviewer</span></label><div className="ctl"><input id={`check-${check}`} className="st-input" value={role} onChange={(e) => field("checks", { ...p.checks, [check]: e.target.value })} /><button type="button" className="g-btn ghost sm" onClick={() => field("checks", Object.fromEntries(Object.entries(p.checks).filter(([id]) => id !== check)))}>Удалить проверку</button></div></div>)}
    <button type="button" className="g-btn ghost sm" onClick={() => { let n = 1; while (`review_${n}` in p.checks) n++; field("checks", { ...p.checks, [`review_${n}`]: "reviewer" }); }}>Добавить проверку</button>
    <div className="st-row"><label htmlFor="intelligence-checks" className="lbl"><b>Проверки для каждой задачи</b><span>Имена через запятую</span></label><div className="ctl"><ListInput id="intelligence-checks" values={p.requiredChecks} onChange={(values) => field("requiredChecks", values)} /></div></div>
    <h4>Защищённые компоненты</h4>
    {p.components.map((c, i) => <div key={i} className="st-row"><div className="lbl"><b>Компонент {i + 1}</b><button type="button" className="g-btn ghost sm" onClick={() => field("components", p.components.filter((_, n) => n !== i))}>Удалить компонент</button></div><div className="ctl">
      <label>Название<input className="st-input" value={c.id} required onChange={(e) => field("components", p.components.map((v, n) => n === i ? { ...v, id: e.target.value } : v))} /></label>
      <label>Пути от корня репозитория, через запятую<ListInput values={c.paths} onChange={(values) => field("components", p.components.map((v, n) => n === i ? { ...v, paths: values } : v))} /></label>
      <label>Чувствительность<select className="st-input" value={c.sensitivity} onChange={(e) => field("components", p.components.map((v, n) => n === i ? { ...v, sensitivity: e.target.value } : v))}>{["security", "privacy", "financial", "protected", "ordinary"].map((s) => <option key={s}>{s}</option>)}</select></label>
      <label>Обязательные проверки, через запятую<ListInput values={c.requiredChecks} onChange={(values) => field("components", p.components.map((v, n) => n === i ? { ...v, requiredChecks: values } : v))} /></label>
    </div></div>)}
    <button type="button" className="g-btn ghost sm" onClick={() => field("components", [...p.components, { id: "", paths: [], sensitivity: "protected", requiredChecks: [] }])}>Добавить компонент</button>
  </section>;
}

function ListInput({ id, values, onChange }: { id?: string; values: string[]; onChange: (values: string[]) => void }) {
  const [text, setText] = useState(values.join(", "));
  return <input id={id} className="st-input" value={text} onChange={(e) => { setText(e.target.value); onChange(split(e.target.value)); }} />;
}
