// «Расходы» in the server's statistics: what the agents' models cost over the period
// (priced with modelPrices of config.json), by day and model, then by project, epic,
// task and chat. Models without a price count their tokens only.

import { useState } from "react";
import { useNavigate } from "react-router";
import { costDays, type ProjectStats, useModelPrices, useRefreshModelPrices } from "@/entities/project";
import { useSession, useSwitchProject } from "@/entities/session";
import {
  chatTitle,
  mergeSpends,
  modelName,
  moneyText,
  type Spend,
  type SpendItem,
  tokensText,
  tokensTotal,
  topItems,
} from "@/entities/usage";
import { BarChart } from "./BarChart.tsx";
import { useAct } from "./ProjectPage.tsx";

const COLORS = ["#7c84f0", "#3aa58a", "#d9a441", "#5ba7e0"];
const OTHER = "#6b6f78";

type View = "projects" | "epics" | "tasks" | "chats";
const VIEWS: { id: View; label: string; hint: string }[] = [
  { id: "projects", label: "Проекты", hint: "все проекты за период" },
  { id: "epics", label: "Эпики", hint: "эпик — сумма его задач" },
  { id: "tasks", label: "Задачи", hint: "самые дорогие задачи, с подзадачами" },
  { id: "chats", label: "Чаты", hint: "разговор одного агента целиком" },
];

type Row = { key: string; name: string; sub: string; extra: string; spend: Spend; to?: { project: string; path: string } };

/** A price per million tokens: «$5», «$0,5». */
const perMillion = (x: number) => `$${String(Math.round(x * 10_000) / 10_000).replace(".", ",")}`;

const priceText = (m: Spend["models"][number]) =>
  m.price
    ? `${perMillion(m.price.input)} / ${perMillion(m.price.output)} / ${m.price.cacheRead === null ? "—" : perMillion(m.price.cacheRead)} / ${m.price.cacheWrite === null ? "—" : perMillion(m.price.cacheWrite)}`
    : "цена не указана";

const tokensTitle = (s: Spend) =>
  `вход ${tokensText(s.tokens.input)}, выход ${tokensText(s.tokens.output)}, из кэша ${tokensText(s.tokens.cacheRead)}, в кэш ${tokensText(s.tokens.cacheWrite)}`;

const modelsText = (s: Spend) =>
  s.models
    .slice(0, 2)
    .map((m) => modelName(m.model))
    .join(", ") + (s.models.length > 2 ? ` и ещё ${s.models.length - 2}` : "");

export function Costs({ list, days }: { list: ProjectStats[]; days: number }) {
  const [view, setView] = useState<View>("epics");
  const act = useAct();
  const modelPrices = useModelPrices();
  const refreshPrices = useRefreshModelPrices();
  const total = mergeSpends(list.map((p) => p.usage?.spend).filter((s): s is Spend => !!s));
  const done = list.reduce((n, p) => n + p.done, 0);
  const unpriced = total.models.filter((m) => m.cost === null || (m.unpricedTokens ?? 0) > 0);
  // The chart: the four most expensive models, the rest as one.
  const top = total.models.filter((m) => m.cost !== null).slice(0, COLORS.length);
  const rest = total.models.filter((m) => m.cost !== null).length > top.length;
  const series = [
    ...top.map((m, i) => ({ key: m.model, label: modelName(m.model), color: COLORS[i], value: (d: ReturnType<typeof costDays>[number]) => d.byModel[m.model] ?? 0 })),
    ...(rest
      ? [{ key: "other", label: "другие", color: OTHER, value: (d: ReturnType<typeof costDays>[number]) => d.cost - top.reduce((n, m) => n + (d.byModel[m.model] ?? 0), 0) }]
      : []),
  ];
  const multi = list.length > 1;
  const many = (p: ProjectStats) => (multi ? `${p.name} · ` : "");

  const rows: Row[] =
    view === "projects"
      ? list.map((p) => ({ key: p.project, name: p.name, sub: p.people.join(", ") || "людей не было", extra: `готово ${p.done}`, spend: p.usage?.spend ?? mergeSpends([]) }))
      : view === "epics"
        ? topItems(
            list.map((p) => ({ project: p.project, items: p.usage?.epics ?? [] })),
            40,
          ).map((e) => {
            const p = list.find((x) => x.project === e.project)!;
            const named = e.id !== "" && e.id !== "-";
            return {
              key: `${e.project}/${e.id}`,
              name: named ? `${e.id} · ${e.title}` : e.id === "" ? "Без эпика" : "Без задачи",
              sub: named ? `${many(p)}эпик` : e.id === "" ? `${many(p)}задачи вне эпиков` : `${many(p)}оркестратор и задания без задачи`,
              extra: modelsText(e.spend),
              spend: e.spend,
              to: named ? { project: e.project, path: `/epic/${encodeURIComponent(e.id)}` } : undefined,
            };
          })
        : view === "tasks"
          ? topItems(
              list.map((p) => ({ project: p.project, items: p.usage?.tasks ?? [] })),
              20,
            ).map((t: SpendItem & { project: string }) => {
              const p = list.find((x) => x.project === t.project)!;
              return {
                key: `${t.project}/${t.id}`,
                name: `${t.id} · ${t.title || "задача удалена"}`,
                sub: `${many(p)}${t.parent ? `эпик ${t.parent}` : "без эпика"}`,
                extra: modelsText(t.spend),
                spend: t.spend,
                to: { project: t.project, path: `/tasks?task=${encodeURIComponent(t.id)}` },
              };
            })
          : topItems(
              list.map((p) => ({ project: p.project, items: p.usage?.chats ?? [] })),
              20,
            ).map((c) => {
              const p = list.find((x) => x.project === c.project)!;
              const t = chatTitle(c.id);
              const member = /^([^/]+)\/([^/]+)$/.exec(c.id);
              return {
                key: `${c.project}/${c.id}`,
                name: t.name,
                sub: `${many(p)}${t.sub}${c.parent && !t.sub.endsWith(c.parent) ? ` · задача ${c.parent}` : ""}`,
                extra: modelsText(c.spend),
                spend: c.spend,
                to: member && !c.id.startsWith("job/") ? { project: c.project, path: `/team/${encodeURIComponent(member[1])}/${encodeURIComponent(member[2])}` } : undefined,
              };
            });
  const sum = rows.reduce((n, r) => n + r.spend.cost, 0);
  const sumTokens = rows.reduce((n, r) => n + tokensTotal(r.spend.tokens), 0);
  const share = (s: Spend) => Math.round(sum > 0 ? (s.cost / sum) * 100 : sumTokens > 0 ? (tokensTotal(s.tokens) / sumTokens) * 100 : 0);
  const cols = { projects: ["Проект", "Готово"], epics: ["Эпик", "Модели"], tasks: ["Задача", "Модели"], chats: ["Чат агента", "Модели"] }[view];

  if (!total.calls) {
    return (
      <section className="st-card sv-pad sv-empty">
        <b>За этот период агенты не тратили токенов.</b>
        <p className="sv-p">
          Расходы считаются с момента обновления сервера: каждый ответ модели агенту попадает в его чат и задачу. Цены моделей задаются в <code>modelPrices</code>{" "}
          файла config.json, в долларах за миллион токенов.
        </p>
      </section>
    );
  }
  return (
    <>
      <div className="sv-tiles">
        <div className="sv-tile">
          <span className="lbl">Потрачено</span>
          <b>{moneyText(total.cost)}</b>
          <span className="sub">
            {modelPrices.data?.fetchedAt
              ? `по тарифам LiteLLM от ${modelPrices.data.fetchedAt.slice(0, 16).replace("T", " ")}, modelPrices перекрывает`
              : "по ценам из config.json"}
          </span>
        </div>
        <div className="sv-tile">
          <span className="lbl">Токенов</span>
          <b title={tokensTitle(total)}>{tokensText(tokensTotal(total.tokens))}</b>
          <span className="sub">
            вход {tokensText(total.tokens.input)} · выход {tokensText(total.tokens.output)} · кэш {tokensText(total.tokens.cacheRead + total.tokens.cacheWrite)}
          </span>
        </div>
        <div className="sv-tile">
          <span className="lbl">Задача в среднем</span>
          <b>{done ? moneyText(total.cost / done) : "—"}</b>
          <span className="sub">{done ? `на каждую из ${done} готовых` : "готовых задач не было"}</span>
        </div>
        <div className={`sv-tile${unpriced.length ? " warn" : ""}`}>
          <span className="lbl">Не оценено</span>
          <b>{unpriced.length ? `${unpriced.length} ${unpriced.length === 1 ? "модель" : unpriced.length < 5 ? "модели" : "моделей"}` : "нет"}</b>
          <span className="sub">{unpriced.length ? `${tokensText(total.unpricedTokens)} токенов не вошли в сумму (модель или кэш без цены)` : "у всех моделей и кэша есть цена"}</span>
        </div>
      </div>
      <div className="sv-charts">
        <BarChart title="Расходы по дням" days={costDays(list)} series={series} stacked format={moneyText} />
        <section className="st-card sv-models" aria-label="По моделям">
          <div className="sv-models-head">
            <b>По моделям</b>
            <span className="muted">цена за 1 млн токенов: вход / выход / кэш чтение / кэш запись</span>
            <button
              type="button"
              className="btn"
              disabled={refreshPrices.isPending}
              onClick={() => void act(() => refreshPrices.mutateAsync(), "Тарифы получены от LiteLLM")}
            >
              Подтянуть тарифы из LiteLLM
            </button>
          </div>
          {modelPrices.data?.lastError ? <p className="sv-cli sv-warn-text">Последняя загрузка тарифов не удалась: {modelPrices.data.lastError}</p> : null}
          <div className="sv-table sv-mtable">
            <div className="tr th">
              <span>Модель</span>
              <span>Цена</span>
              <span>Токены</span>
              <span className="num">Стоимость</span>
            </div>
            {total.models.map((m, i) => (
              <div key={m.model} className="tr">
                <span className="nm sv-model">
                  <i style={{ background: m.cost === null ? "var(--line-strong)" : (COLORS[i] ?? OTHER) }} />
                  <span className="sv-mono" title={m.model}>
                    {m.model}
                  </span>
                </span>
                <span data-l="Цена" className={m.price ? "muted" : "sv-warn-text"}>
                  {priceText(m)}
                </span>
                <span data-l="Токены" title={tokensTitle({ ...total, tokens: m.tokens })}>
                  {tokensText(tokensTotal(m.tokens))}
                  {(m.unpricedTokens ?? 0) > 0 ? <span className="muted"> · {tokensText(m.unpricedTokens)} без цены</span> : null}
                </span>
                <span data-l="Стоимость" className="num">
                  {m.cost === null ? "—" : moneyText(m.cost)}
                </span>
              </div>
            ))}
          </div>
          <p className="sv-cli sv-pad">
            Тарифы берутся из LiteLLM (кнопка «Подтянуть тарифы», то же — <code>genie server prices --refresh</code>), а <code>modelPrices</code> файла config.json перекрывает их по полям. Кэш без цены не считается по цене входа: такие токены показываются отдельно.
          </p>
        </section>
      </div>
      <SpendTable view={view} setView={setView} cols={cols} rows={rows} share={share} />
      <p className="sv-cli">
        То же в терминале: <code>genie stats --days {days}</code>
      </p>
    </>
  );
}

function SpendTable({
  view,
  setView,
  cols,
  rows,
  share,
}: {
  view: View;
  setView: (v: View) => void;
  cols: string[];
  rows: Row[];
  share: (s: Spend) => number;
}) {
  const navigate = useNavigate();
  const session = useSession().data;
  const switchProject = useSwitchProject();
  const open = async (to: NonNullable<Row["to"]>) => {
    if (session?.project !== to.project) await switchProject(to.project);
    navigate(to.path);
  };
  return (
    <section className="st-card" aria-label="На что ушло">
      <div className="sv-models-head">
        <b>На что ушло</b>
        <div className="seg" role="group" aria-label="Разрез">
          {VIEWS.map((v) => (
            <button key={v.id} type="button" className={v.id === view ? "on" : ""} aria-pressed={v.id === view} onClick={() => setView(v.id)}>
              {v.label}
            </button>
          ))}
        </div>
        <span className="muted sv-hint">{VIEWS.find((v) => v.id === view)!.hint}</span>
      </div>
      <div className="sv-table sv-stable">
        <div className="tr th">
          <span>{cols[0]}</span>
          <span>{cols[1]}</span>
          <span>Токены</span>
          <span>Доля</span>
          <span className="num">Стоимость</span>
        </div>
        {rows.length === 0 && <p className="muted sv-pad">Пусто.</p>}
        {rows.map((r) => (
          <div key={r.key} className="tr">
            <span className="nm">
              {r.to ? (
                <a
                  href={r.to.path}
                  onClick={(e) => {
                    e.preventDefault();
                    void open(r.to!);
                  }}
                >
                  {r.name}
                </a>
              ) : (
                <b>{r.name}</b>
              )}
              <span className="muted">{r.sub}</span>
            </span>
            <span data-l={cols[1]} className="muted sv-clip" title={r.extra}>
              {r.extra}
            </span>
            <span data-l="Токены" title={tokensTitle(r.spend)}>
              {tokensText(tokensTotal(r.spend.tokens))}
            </span>
            <span data-l="Доля" className="sv-share">
              <i>
                <i style={{ width: `${share(r.spend)}%` }} />
              </i>
              <em>{share(r.spend)}%</em>
            </span>
            <span data-l="Стоимость" className="num">
              {r.spend.models.some((m) => m.cost !== null) ? moneyText(r.spend.cost) : "—"}
            </span>
          </div>
        ))}
      </div>
    </section>
  );
}
