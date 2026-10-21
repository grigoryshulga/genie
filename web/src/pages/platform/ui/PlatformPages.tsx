// Pages for the platform features of the server: answering questions (public
// link), notifications, using the automations (the rules themselves are defined
// in the settings mode), and the review of knowledge proposals.

import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { type Automation, type Notification, describeTrigger, RUN_STATUS, useAutomations, useNotifications, useProposal, useProposals } from "@/entities/platform";
import { request } from "@/shared/api";
import { matchesSearch, timeAgo, useTick } from "@/shared/lib";
import { Banner, Button, Card, Character, EmptyState, GroupHead, Icon, Markdown, Prose, Radio, Sheet, Skeleton, Tag, Textarea, useRefreshingAct, useToast, WorkingArcs } from "@/shared/ui";
import { ContextSearch } from "@/widgets/topbar";
import { Runs } from "@/widgets/runs";

// ------------------------------------------------------------------ answer page (public)

interface Questionnaire {
  id: number;
  task?: string;
  askedBy: string;
  status: string;
  questions: { n: number; text: string; why: string; options: string[]; answer?: string }[];
}

export function AnswerPage() {
  const [sp] = useSearchParams();
  const token = sp.get("token") ?? "";
  const [data, setData] = useState<{ questionnaire: Questionnaire; taskTitle?: string } | null>();
  const [error, setError] = useState<string>();
  const [answers, setAnswers] = useState<Record<number, string>>({});
  const [done, setDone] = useState(false);
  if (data === undefined && !error) {
    request<{ questionnaire: Questionnaire; taskTitle?: string }>("GET", `/api/answer/${encodeURIComponent(token)}`).then(setData, (e: Error) => setError(e.message));
  }
  const submit = async () => {
    try {
      const res = await request<{ questionnaire: Questionnaire; complete: boolean }>("POST", `/api/answer/${encodeURIComponent(token)}`, { answers });
      setData((d) => (d ? { ...d, questionnaire: res.questionnaire } : d));
      setDone(res.complete);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  const qn = data?.questionnaire;
  const closed = done || (qn && qn.status !== "open");
  return (
    <main className="answer-page">
      <div className="answer-body">
        {error && (
          <>
            <div className="answer-head">
              <div className="answer-head-copy">
                <span className="answer-ctx">вопросы к вам · ссылка из уведомления</span>
                <h1>Ссылка не открылась</h1>
                <p className="answer-lead">Проверьте, что она скопирована целиком: ссылка живёт, пока вопрос открыт.</p>
              </div>
              <Character state="sleep" size={148} />
            </div>
            <Banner tone="danger" title="Не получилось открыть вопросы">
              {error}
            </Banner>
          </>
        )}
        {!qn && !error && (
          <div className="answer-loading">
            <WorkingArcs size={22} label="Загружаем вопросы" />
            <span className="g-caption">Загружаем вопросы…</span>
          </div>
        )}
        {qn && (
          <>
            <div className="answer-head">
              <div className="answer-head-copy">
                <span className="answer-ctx">
                  {qn.task ? <span className="g-mono">{qn.task}</span> : "genie"} · спрашивает {qn.askedBy}
                </span>
                <h1>
                  {closed ? (
                    "Ответы получены"
                  ) : (
                    <>
                      Нужны ваши ответы
                      <span className="g-stop">.</span>
                    </>
                  )}
                </h1>
                {data?.taskTitle && <p className="answer-lead">{data.taskTitle}</p>}
              </div>
              <Character state={closed ? "sleep" : "surprise"} size={148} />
            </div>
            {closed ? (
              <Sheet>
                <p className="answer-done">
                  {qn.status === "expired" ? "Срок ответа истёк — вопрос передан владельцу проекта." : "Спасибо! Ответы записаны в задачу, команда продолжит работу."}
                </p>
              </Sheet>
            ) : (
              <form
                className="answer-form"
                onSubmit={(e) => {
                  e.preventDefault();
                  void submit();
                }}
              >
                {qn.questions.map((q) => (
                  <Sheet key={q.n}>
                    <div className="answer-question">
                      <h2 className="answer-legend">
                        <span className="answer-num">{q.n}.</span> {q.text}
                      </h2>
                      {q.why && <p className="answer-why">{q.why}</p>}
                      {q.answer ? (
                        <p className="answer-answered">
                          <Icon.check size={13} />
                          {q.answer}
                        </p>
                      ) : (
                        <fieldset className="answer-options">
                          <legend className="g-sr">Ответ на вопрос {q.n}</legend>
                          {q.options.map((o) => (
                            <Radio key={o} name={`q${q.n}`} label={o} checked={answers[q.n] === o} onChange={() => setAnswers({ ...answers, [q.n]: o })} />
                          ))}
                          <Textarea
                            rows={q.options.length ? 2 : 3}
                            aria-label={`Ответ на вопрос ${q.n}`}
                            value={q.options.includes(answers[q.n] ?? "") ? "" : (answers[q.n] ?? "")}
                            placeholder={q.options.length ? "Или свой ответ" : "Ваш ответ"}
                            onChange={(e) => setAnswers({ ...answers, [q.n]: e.target.value })}
                          />
                        </fieldset>
                      )}
                    </div>
                  </Sheet>
                ))}
                <div className="answer-actions">
                  <Button type="submit" variant="primary">
                    Отправить ответы
                  </Button>
                  <span className="answer-note">Вход не нужен — ссылка работает, пока вопрос открыт</span>
                </div>
              </form>
            )}
          </>
        )}
      </div>
    </main>
  );
}

// ------------------------------------------------------------------ notifications

/** The type of a notification in one icon, so a list of many reads by shape first. */
function notificationIcon(kind: string) {
  if (kind.startsWith("task.comment")) return <Icon.send size={14} />;
  if (kind.startsWith("task.merge") || kind.startsWith("task.pr")) return <Icon.merge size={14} />;
  if (kind.startsWith("task.status")) return <Icon.bolt size={14} />;
  if (kind.startsWith("proposal")) return <Icon.proposal size={14} />;
  if (kind.startsWith("doc")) return <Icon.file size={14} />;
  if (kind.startsWith("agent") || kind.startsWith("team")) return <Icon.user size={14} />;
  if (kind.startsWith("observation")) return <Icon.info size={14} />;
  return <Icon.bell size={14} />;
}

export function NotificationsPage() {
  useTick();
  const q = useNotifications();
  const act = useRefreshingAct();
  const navigate = useNavigate();
  const [search] = useSearchParams();
  const query = search.get("q") ?? "";
  const items = (q.data?.items ?? []).filter((n) => matchesSearch(query, n.title, n.body));
  const days: { day: string; items: Notification[] }[] = [];
  for (const n of items) {
    const day = dayName(n.created);
    if (days.at(-1)?.day !== day) days.push({ day, items: [] });
    days.at(-1)!.items.push(n);
  }
  const unread = q.data?.unread ?? 0;
  return (
    <main className="main">
      <header className="topbar">
        <h1>Уведомления</h1>
        {unread > 0 && <span className="sub">{unread} непрочитанных</span>}
        <span className="grow" />
        {unread > 0 && (
          <Button onClick={() => void act(() => request("POST", "/api/notifications/read", {}))}>Прочитать все</Button>
        )}
      </header>
      <ContextSearch className="mobile-context-search mobile-search-bar" />
      <div className="scroll">
        {q.isPending ? (
          <div className="notifs" aria-hidden="true">
            <div className="g-card flush notif-skeletons">
              {[0, 1, 2, 3].map((i) => (
                <div className="notif-row" key={i}>
                  <Skeleton width={14} height={14} />
                  <Skeleton width={`${58 - i * 7}%`} />
                </div>
              ))}
            </div>
          </div>
        ) : !items.length ? (
          <div className="notif-empty">
            <Character state="sleep" size={132} />
            {query ? (
              <EmptyState title="Ничего не найдено">Измените запрос или очистите поиск.</EmptyState>
            ) : (
              <EmptyState title="Уведомлений нет">Здесь появятся ответы агентов, решения владельцев и упоминания.</EmptyState>
            )}
          </div>
        ) : (
          <div className="notifs">
            {days.map((d) => (
              <section key={d.day} className="notif-day">
                <GroupHead count={d.items.length}>{d.day}</GroupHead>
                <Card flush>
                  {d.items.map((n) => (
                    <button
                      type="button"
                      key={n.id}
                      className="g-row notif-row"
                      data-unread={n.readAt ? undefined : "true"}
                      onClick={() =>
                        void act(async () => {
                          if (!n.readAt) await request("POST", "/api/notifications/read", { id: n.id });
                          if (n.link) navigate(n.link);
                        })
                      }
                    >
                      <span className="notif-dot" aria-hidden="true" />
                      {!n.readAt && <span className="g-sr">не прочитано</span>}
                      <span className="notif-kind" aria-hidden="true">
                        {notificationIcon(n.kind)}
                      </span>
                      <span className="notif-text">
                        <b>{n.title}</b>
                        {n.body && <span className="notif-body">{n.body}</span>}
                      </span>
                      <span className="notif-when g-caption">{timeAgo(n.created)}</span>
                    </button>
                  ))}
                </Card>
              </section>
            ))}
          </div>
        )}
      </div>
    </main>
  );
}

function dayName(iso: string): string {
  const d = new Date(iso);
  const start = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((start(new Date()) - start(d)) / 86_400_000);
  if (diff <= 0) return "Сегодня";
  if (diff === 1) return "Вчера";
  return d.toLocaleDateString("ru-RU", { day: "numeric", month: "long" });
}

// ------------------------------------------------------------------ automations

/** Using the automations: the rules with their last run, a manual run, and the run journal. */
export function AutomationsPage() {
  useTick();
  const rules = useAutomations();
  const [sp, setSp] = useSearchParams();
  const act = useRefreshingAct();
  const selected = Number(sp.get("rule") ?? "") || undefined;
  const filtered = rules.data?.filter((r) => matchesSearch(sp.get("q") ?? "", r.name, describeTrigger(r.spec, r.enabled, r.dryRun)));
  const current = filtered?.find((r) => r.id === selected);
  const selectRule = (id?: number) => setSp((previous) => {
    const next = new URLSearchParams(previous);
    if (id) next.set("rule", String(id)); else next.delete("rule");
    return next;
  });
  const runNow = (rule: Automation) => void act(() => request("POST", `/api/automations/${rule.id}/run`, {}), "Запущено");
  return (
    <main className="main">
      <header className="topbar">
        <h1>Автоматизации</h1>
        <span className="sub">запуск и наблюдение · правила меняют в настройках проекта</span>
        <span className="grow" />
        {current?.trigger === "manual" && (
          <Button variant="primary" onClick={() => runNow(current)}>
            <Icon.bolt size={13} />
            <span className="d-only">Запустить</span>
          </Button>
        )}
      </header>
      <ContextSearch className="mobile-context-search mobile-search-bar" />
      <div className="split">
        <section className="split-list" aria-label="Правила">
          {rules.isPending && (
            <div className="split-skeletons" aria-hidden="true">
              {[0, 1, 2].map((i) => (
                <Skeleton key={i} height={44} />
              ))}
            </div>
          )}
          {rules.isError && <Banner tone="danger" title="Не получилось прочитать правила">{rules.error.message}</Banner>}
          {rules.data?.length ? (
            <button type="button" className="g-row split-row" aria-selected={!current} onClick={() => selectRule()}>
              <Icon.list size={13} />
              <span className="split-row-text">
                <span className="split-row-title">Все правила</span>
                <span className="split-row-sub">общий журнал</span>
              </span>
            </button>
          ) : null}
          {filtered?.map((r) => (
            <button type="button" key={r.id} className="g-row split-row" aria-selected={r.id === current?.id} onClick={() => selectRule(r.id)}>
              <span className={`state-dot ${r.enabled ? "ok" : "off"}`} aria-hidden="true" />
              <span className="split-row-text">
                <span className="split-row-title">{r.name}</span>
                <span className="split-row-sub">{describeTrigger(r.spec, r.enabled, r.dryRun)}</span>
                <span className="split-row-sub">
                  {r.lastRun ? `последний запуск ${timeAgo(r.lastRun.started)} · ${(RUN_STATUS[r.lastRun.status] ?? r.lastRun.status).toLowerCase()}` : "ещё не запускалось"}
                </span>
              </span>
            </button>
          ))}
          {rules.isSuccess && rules.data.length > 0 && !filtered?.length && <p className="split-hint">Ничего не найдено</p>}
          {rules.isSuccess && !rules.data.length && (
            <p className="split-hint">Правил пока нет — их добавляют в настройках проекта, на разделе «Автоматизации».</p>
          )}
        </section>
        <section className="split-main" aria-label="Запуски">
          <div className="pane-head">
            <div className="ttl">
              <h2>{current ? current.name : "Все правила"}</h2>
              <span className="sub">{current ? describeTrigger(current.spec, current.enabled, current.dryRun) : "журнал запусков всех правил"}</span>
            </div>
            {current?.trigger === "manual" && <Button onClick={() => runNow(current)}>Запустить</Button>}
          </div>
          <div className="pane-body">
            <Runs automation={current?.id} />
          </div>
        </section>
      </div>
    </main>
  );
}

// ------------------------------------------------------------------ knowledge proposals

/** Page body with the frontmatter shown apart, so the comparison reads like the page. */
function PageBody({ text }: { text: string }) {
  const m = /^---\n([\s\S]*?)\n---\n?/.exec(text);
  return (
    <>
      {m && <pre className="frontmatter">{m[1]}</pre>}
      <Prose>
        <Markdown text={m ? text.slice(m[0].length) : text} />
      </Prose>
    </>
  );
}

export function ProposalsPage() {
  useTick();
  const [sp, setSp] = useSearchParams();
  const list = useProposals();
  const filtered = list.data?.filter((p) => matchesSearch(sp.get("q") ?? "", p.path, p.author, p.task));
  const selected = Number(sp.get("proposal") ?? "") || filtered?.[0]?.id;
  const one = useProposal(selected);
  const act = useRefreshingAct();
  const decide = (verb: "approve" | "reject", force = false) =>
    act(async () => {
      await request("POST", `/api/docs/proposals/${selected}/${verb}`, { force });
      setSp({});
    }, verb === "approve" ? "Правка опубликована" : "Предложение отклонено");
  const p = one.data?.proposal;
  return (
    <main className="main">
      <header className="topbar">
        <h1>Предложения</h1>
        <span className="sub d-only">агенты предлагают правки знаний, владельцы раздела решают</span>
        <span className="grow" />
        <Link className="g-btn secondary" to="/docs">
          К документации
        </Link>
      </header>
      <ContextSearch className="mobile-context-search mobile-search-bar" />
      <div className="split narrow">
        <section className="split-list" aria-label="Открытые предложения">
          {list.isPending && (
            <div className="split-skeletons" aria-hidden="true">
              {[0, 1, 2].map((i) => (
                <Skeleton key={i} height={44} />
              ))}
            </div>
          )}
          {list.isError && <Banner tone="danger" title="Не получилось прочитать предложения">{list.error.message}</Banner>}
          {list.isSuccess && !list.data.length && <p className="split-hint">Открытых предложений нет.</p>}
          {list.isSuccess && list.data.length > 0 && !filtered?.length && <p className="split-hint">Ничего не найдено</p>}
          {filtered?.map((x) => (
            <button
              type="button"
              key={x.id}
              className="g-row split-row"
              aria-selected={selected === x.id}
              onClick={() => setSp((previous) => { const next = new URLSearchParams(previous); next.set("proposal", String(x.id)); return next; })}
            >
              <Icon.proposal size={13} />
              <span className="split-row-text">
                <span className="split-row-title">{pageTitle(x.path)}</span>
                <span className="split-row-sub">
                  {x.author}
                  {x.authorKind === "agent" ? " (агент)" : ""}
                  {x.task ? ` · по задаче ${x.task}` : ""} · {timeAgo(x.created)}
                </span>
              </span>
            </button>
          ))}
        </section>
        <section className="split-main" aria-label="Предложение">
          {!p ? (
            <div className="pane-empty">
              <Icon.proposal size={22} />
              {list.isPending || (selected && one.isPending) ? (
                <>
                  <WorkingArcs size={22} label="Загружаем предложение" />
                  <span>Загружаем предложение…</span>
                </>
              ) : (
                <span>Здесь появятся правки, которые ждут вашего решения</span>
              )}
            </div>
          ) : (
            <>
              <div className="pane-head">
                <div className="ttl">
                  <h2>{pageTitle(p.path)}</h2>
                  <span className="sub">
                    <span className="g-mono">{p.path}</span>
                    {one.data!.owners.length > 0 ? ` · владельцы раздела: ${one.data!.owners.join(", ")}` : ""}
                  </span>
                </div>
                <Button onClick={() => void decide("reject")}>Отклонить</Button>
                <Button onClick={() => void decide("approve", true)} title="Если страница изменилась после предложения">
                  Опубликовать поверх
                </Button>
                <Button variant="primary" onClick={() => void decide("approve")}>
                  Опубликовать
                </Button>
              </div>
              <div className="proposal-state">
                <Tag tone={one.data!.current == null ? "accent" : "neutral"}>{one.data!.current == null ? "Новая страница" : "Правка страницы"}</Tag>
                <span>{one.data!.current == null ? "страницы ещё нет — предложение создаст её" : "при конфликте сервер попросит опубликовать поверх"}</span>
              </div>
              {p.note && <p className="proposal-note">{p.note}</p>}
              <div className="compare">
                <div>
                  <span className="hd">Сейчас</span>
                  <div className="bd">{one.data!.current ? <PageBody text={one.data!.current} /> : <p className="muted">Страницы ещё нет</p>}</div>
                </div>
                <div>
                  <span className="hd">Предложение</span>
                  <div className="bd">
                    <PageBody text={p.content ?? ""} />
                  </div>
                </div>
              </div>
            </>
          )}
        </section>
      </div>
    </main>
  );
}

function pageTitle(path: string): string {
  return path.replace(/\.md$/, "").split("/").join(" / ");
}
