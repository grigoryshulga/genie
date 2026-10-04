import { useState } from "react";
import { Link, useNavigate } from "react-router";
import { EpicIcon } from "@/entities/task";
import { clock } from "@/shared/lib";
import { Icon, useToast } from "@/shared/ui";
import { useApplyIdea, useIdeaPlan } from "../api.ts";
import { applyLabel, depLabel, IDEA_LABEL, planToApply } from "../model.ts";
import "./idea.css";

const TYPE_RU: Record<string, string> = { task: "задача", bug: "баг", spike: "исследование" };

/**
 * The plan of an idea next to the planner's chat: the epic, the tasks with
 * their criteria and dependencies, and one button that files them. Unticked
 * tasks stay out; everything else is changed by asking the planner.
 */
export function IdeaPlan({ taskId, planner, onClose }: { taskId: string; planner: string; onClose?: () => void }) {
  const { task, plan, version, at, loading, unreadable } = useIdeaPlan(taskId);
  const apply = useApplyIdea();
  const toast = useToast();
  const navigate = useNavigate();
  const [off, setOff] = useState<Set<string>>(new Set());
  const [open, setOpen] = useState<string | undefined>();
  const filed = !!task && !task.labels.includes(IDEA_LABEL);

  const flip = (key: string) => setOff((s) => (s.has(key) ? new Set([...s].filter((k) => k !== key)) : new Set([...s, key])));
  const left = plan ? plan.tasks.filter((t) => !off.has(t.key)).length : 0;
  const submit = () => {
    if (!plan || !left) return;
    apply.mutate(
      { id: taskId, plan: planToApply(plan, off) },
      {
        onSuccess: (r) => {
          toast(`${r.epic ? `Эпик ${r.id} и задачи ${r.created.join(", ")}` : [r.id, ...r.created].join(", ")} во входящих · оркестратор уведомлён`);
          navigate(r.epic ? `/epic/${encodeURIComponent(r.id)}` : `/tasks?task=${encodeURIComponent(r.id)}`);
        },
        onError: (e) => toast(`Не заведено: ${e.message}`, "error"),
      },
    );
  };

  return (
    <div className="idea-plan">
      <div className="ip-hd">
        <b>План</b>
        {version > 0 && (
          <span className="muted">
            версия {version}
            {at ? ` · ${clock(at)}` : ""}
          </span>
        )}
        <span className="grow" />
        {plan && !filed && (
          <span className="muted">
            {left} из {plan.tasks.length} к заведению
          </span>
        )}
        {onClose && (
          <button type="button" className="icon-btn m-only" aria-label="Закрыть план" onClick={onClose}>
            <Icon.close />
          </button>
        )}
      </div>

      <div className="ip-body">
        {filed ? (
          <div className="ip-note ok">
            <b>План заведён</b>
            <span>
              {task.type === "epic" ? "Идея стала эпиком, задачи во входящих у оркестратора." : "Задачи во входящих у оркестратора."}{" "}
              <Link to={task.type === "epic" ? `/epic/${encodeURIComponent(task.id)}` : `/tasks?task=${encodeURIComponent(task.id)}`}>Открыть {task.id}</Link>
            </span>
          </div>
        ) : !plan ? (
          <div className="ip-note">
            <b>{loading ? "Загрузка плана…" : unreadable ? "План не читается" : "Плана пока нет"}</b>
            <span>
              {unreadable
                ? `${planner} сохранил план в неверном виде. Попросите его сохранить план ещё раз.`
                : `${planner} сначала задаст пару вопросов, потом предложит задачи. План появится здесь и будет обновляться по ходу разговора.`}
            </span>
          </div>
        ) : (
          <>
            {plan.epic && (
              <section className="ip-epic">
                <span className="kind">
                  <EpicIcon size={13} /> Эпик
                </span>
                <b>{plan.epic.title}</b>
                {plan.epic.goal && <p>{plan.epic.goal}</p>}
                {plan.epic.criteria.length > 0 && (
                  <ul aria-label="Критерии успеха">
                    {plan.epic.criteria.map((c) => (
                      <li key={c}>{c}</li>
                    ))}
                  </ul>
                )}
              </section>
            )}
            <ol className="ip-tasks" aria-label="Задачи">
              {plan.tasks.map((t, i) => {
                const on = !off.has(t.key);
                const expanded = open === t.key;
                return (
                  <li key={t.key} className={`ip-task${on ? "" : " off"}`}>
                    <input type="checkbox" checked={on} onChange={() => flip(t.key)} aria-label={`${on ? "Не заводить" : "Заводить"}: ${t.title}`} />
                    <div className="ip-main">
                      <button type="button" className="ip-title" aria-expanded={expanded} onClick={() => setOpen(expanded ? undefined : t.key)}>
                        <span className="n">{i + 1}</span>
                        <span className="t">{t.title}</span>
                        <Icon.chevron size={10} />
                      </button>
                      {!expanded && t.description && <span className="ip-desc">{t.description}</span>}
                      {expanded && (
                        <div className="ip-more">
                          {t.description && <p>{t.description}</p>}
                          {t.criteria.length > 0 && (
                            <ul aria-label="Критерии приёмки">
                              {t.criteria.map((c) => (
                                <li key={c}>{c}</li>
                              ))}
                            </ul>
                          )}
                        </div>
                      )}
                      <span className="ip-tags">
                        <span className="tag">
                          {t.criteria.length ? `${t.criteria.length} ${t.criteria.length === 1 ? "критерий" : t.criteria.length < 5 ? "критерия" : "критериев"}` : "без критериев"}
                        </span>
                        {t.deps.length > 0 && <span className={`tag dep${t.deps.some((d) => off.has(d)) ? " lost" : ""}`}>после {t.deps.map((d) => depLabel(plan, d)).join(", ")}</span>}
                        {t.type !== "task" && <span className="tag">{TYPE_RU[t.type]}</span>}
                      </span>
                    </div>
                  </li>
                );
              })}
            </ol>
            {(plan.assumptions.length > 0 || plan.questions.length > 0) && (
              <section className="ip-open">
                {plan.questions.length > 0 && (
                  <>
                    <h3>Открытые вопросы</h3>
                    <ul>
                      {plan.questions.map((q) => (
                        <li key={q}>{q}</li>
                      ))}
                    </ul>
                  </>
                )}
                {plan.assumptions.length > 0 && (
                  <>
                    <h3>Допущения</h3>
                    <ul>
                      {plan.assumptions.map((q) => (
                        <li key={q}>{q}</li>
                      ))}
                    </ul>
                  </>
                )}
              </section>
            )}
          </>
        )}
      </div>

      {plan && !filed && (
        <div className="ip-ft">
          <button type="button" className="btn primary" onClick={submit} disabled={!left || apply.isPending}>
            {apply.isPending ? "Заводим…" : applyLabel(!!plan.epic, left)}
          </button>
          <span className="ac-help">
            {plan.epic ? `${taskId} станет эпиком` : `${taskId} станет первой задачей`}, задачи попадут во входящие с зависимостями, оркестратор возьмёт их как обычно. Разговор закончится, исходная идея сохранится в заметках.
          </span>
        </div>
      )}
    </div>
  );
}
