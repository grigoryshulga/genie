// The observation sources of the project (docs/platform/observation-plane.md §13.2, §15, §17):
// the servers that observation cases read their measurements from. Only project administrators
// reach the list; the server refuses everyone else, so other members see a notice instead.
// A secret is entered once, never shown again, and only its hint is listed.

import { type FormEvent, useId, useState } from "react";
import {
  OBSERVATION_TEXT,
  useAddSource,
  useClearSourceSecret,
  useObservationSources,
  useRemoveSource,
  useSetSourceSecret,
  useTestSource,
  useUpdateSource,
} from "@/entities/observation";
import type { SourceAuth } from "@/shared/api/generated/SourceAuth.ts";
import type { SourceBody } from "@/shared/api/generated/SourceBody.ts";
import type { SourceKind } from "@/shared/api/generated/SourceKind.ts";
import type { SourceTestView } from "@/shared/api/generated/SourceTestView.ts";
import type { SourceView } from "@/shared/api/generated/SourceView.ts";
import { ConfirmDialog, Icon, useAct } from "@/shared/ui";

const SOURCE_KINDS: SourceKind[] = ["prometheus", "sentry"];
const SOURCE_AUTHS: SourceAuth[] = ["none", "bearer", "basic"];

const KIND_LABEL: Record<SourceKind, string> = {
  prometheus: "Prometheus-совместимый",
  sentry: "Sentry-совместимый",
};

const AUTH_LABEL: Record<SourceAuth, string> = {
  none: "Без авторизации",
  bearer: "Токен (Bearer)",
  basic: "Логин и пароль (Basic)",
};

const QUERY_PLACEHOLDER: Record<SourceKind, string> = {
  prometheus: "sum(rate(http_requests_total[$__range]))",
  sentry: "level:error",
};

/** The source name rule of §13.2; the server also refuses the reserved name `manual`. */
const NAME_PATTERN = "[a-z0-9][a-z0-9_\\-]{0,31}";

export function ObservationSources({ admin }: { admin: boolean }) {
  const sources = useObservationSources(admin);
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<string>();
  const list = sources.data ?? [];

  return (
    <div className="st-body">
      <div className="st-head">
        <h2>{OBSERVATION_TEXT.section}</h2>
        <p>Источники — серверы метрик и ошибок, из которых наблюдения берут измерения. Секреты хранятся на сервере зашифрованными и после сохранения не показываются.</p>
      </div>

      {!admin ? (
        <section className="st-card" aria-label="Доступ к источникам">
          <div className="st-card-body">
            <b>Только для администраторов проекта</b>
            <p className="muted">Источники наблюдения и их секреты настраивают администраторы проекта. Попросите администратора подключить источник или проверить запрос.</p>
          </div>
        </section>
      ) : (
        <>
          {!adding && (
            <button type="button" className="btn primary" style={{ alignSelf: "flex-start" }} onClick={() => setAdding(true)}>
              <Icon.plus size={14} />
              Добавить источник
            </button>
          )}
          {adding && (
            <section className="st-card" aria-label="Новый источник">
              <div className="st-card-head">
                <h3>Новый источник</h3>
              </div>
              <SourceForm onDone={() => setAdding(false)} />
            </section>
          )}

          {sources.isPending ? (
            <p className="muted">Загрузка…</p>
          ) : sources.error ? (
            <div role="alert" className="st-note">
              {sources.error.message}{" "}
              <button type="button" className="btn sm" onClick={() => void sources.refetch()}>
                Повторить
              </button>
            </div>
          ) : list.length > 0 ? (
            list.map((source) => (
              <SourceCard key={source.name} source={source} editing={editing === source.name} onEdit={(on) => setEditing(on ? source.name : undefined)} />
            ))
          ) : (
            !adding && (
              <section className="st-card" aria-label="Источников нет">
                <div className="st-card-body">
                  <b>Источников пока нет</b>
                  <p className="muted">Наблюдение берёт измерения из источников. Добавьте Prometheus или Sentry, чтобы проверять результат поставок.</p>
                </div>
              </section>
            )
          )}
        </>
      )}
    </div>
  );
}

function SourceCard({ source, editing, onEdit }: { source: SourceView; editing: boolean; onEdit: (on: boolean) => void }) {
  const remove = useRemoveSource();
  const act = useAct();
  const [removing, setRemoving] = useState(false);
  const titleId = useId();
  return (
    <section className="st-card" aria-labelledby={titleId}>
      <div className="st-card-head" style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
        <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", gap: 3 }}>
          <h3 id={titleId} className="mono">
            {source.name}
          </h3>
          <p>
            {KIND_LABEL[source.kind]} · <span className="mono">{source.url}</span>
          </p>
        </div>
        {!editing && (
          <button type="button" className="btn sm" onClick={() => onEdit(true)}>
            Изменить
          </button>
        )}
        <button type="button" className="icon-btn" aria-label={`Удалить источник ${source.name}`} title="Удалить источник" onClick={() => setRemoving(true)}>
          <Icon.trash size={13} />
        </button>
      </div>

      {editing ? (
        <SourceForm source={source} onDone={() => onEdit(false)} />
      ) : (
        <div className="st-card-body">
          <dl className="st-facts">
            <dt>Авторизация</dt>
            <dd>{AUTH_LABEL[source.auth]}</dd>
            <dt>Секрет</dt>
            <dd>{source.secret ? <>Задан: <span className="mono">{source.secret.hint}</span></> : "Не задан"}</dd>
            {source.kind === "sentry" && (
              <>
                <dt>Организация</dt>
                <dd>{source.organization}</dd>
                <dt>Проект Sentry</dt>
                <dd>{source.sentryProject ?? "все проекты организации"}</dd>
                <dt>Окружение</dt>
                <dd>{source.environment ?? "все окружения"}</dd>
                <dt>Поле</dt>
                <dd>{source.field ?? "count() по умолчанию"}</dd>
                <dt>Набор данных</dt>
                <dd>{source.dataset ?? "errors по умолчанию"}</dd>
              </>
            )}
            <dt>Таймаут</dt>
            <dd>{source.timeoutSecs !== undefined ? `${source.timeoutSecs} с` : "30 с по умолчанию"}</dd>
          </dl>
        </div>
      )}

      <SecretRow source={source} />
      <TestRow source={source} />

      {removing && (
        <ConfirmDialog
          title="Удалить источник?"
          confirmLabel="Удалить"
          danger
          busy={remove.isPending}
          onClose={() => setRemoving(false)}
          onConfirm={() =>
            void act(() => remove.mutateAsync(source.name), `Источник ${source.name} удалён`).then((ok) => {
              if (ok) {
                setRemoving(false);
                onEdit(false);
              }
            })
          }
        >
          Секрет источника удалится вместе с ним. Наблюдения, которые его используют, перестанут получать измерения: их сбор будет записывать ошибку. Вернуть источник можно,
          создав его заново с тем же именем.
        </ConfirmDialog>
      )}
    </section>
  );
}

/** The editable fields of a source. Numbers and optional text stay strings until the body is built. */
interface Draft {
  name: string;
  kind: SourceKind;
  url: string;
  auth: SourceAuth;
  organization: string;
  sentryProject: string;
  environment: string;
  field: string;
  dataset: string;
  timeoutSecs: string;
  /** Only read when a new source is saved; an existing one changes its secret in its own row. */
  secret: string;
}

const EMPTY_DRAFT: Draft = {
  name: "",
  kind: "prometheus",
  url: "",
  auth: "none",
  organization: "",
  sentryProject: "",
  environment: "",
  field: "",
  dataset: "",
  timeoutSecs: "",
  secret: "",
};

function draftOf(source: SourceView): Draft {
  return {
    name: source.name,
    kind: source.kind,
    url: source.url,
    auth: source.auth,
    organization: source.organization ?? "",
    sentryProject: source.sentryProject ?? "",
    environment: source.environment ?? "",
    field: source.field ?? "",
    dataset: source.dataset ?? "",
    timeoutSecs: source.timeoutSecs === undefined ? "" : String(source.timeoutSecs),
    secret: "",
  };
}

/**
 * The request body of a draft. Fields that the kind does not use are left out, so an update
 * drops them: the server replaces the whole configuration (§13.2).
 */
function bodyOf(draft: Draft): SourceBody {
  const text = (value: string) => value.trim() || undefined;
  const sentry = draft.kind === "sentry";
  return {
    name: draft.name.trim(),
    kind: draft.kind,
    url: draft.url.trim(),
    auth: draft.auth,
    organization: sentry ? text(draft.organization) : undefined,
    sentryProject: sentry ? text(draft.sentryProject) : undefined,
    environment: sentry ? text(draft.environment) : undefined,
    field: sentry ? text(draft.field) : undefined,
    dataset: sentry ? text(draft.dataset) : undefined,
    timeoutSecs: text(draft.timeoutSecs) === undefined ? undefined : Number(draft.timeoutSecs),
  };
}

/** The form of a new source (no `source`) or of an existing one, whose name cannot change. */
function SourceForm({ source, onDone }: { source?: SourceView; onDone: () => void }) {
  const editing = source !== undefined;
  const add = useAddSource();
  const update = useUpdateSource();
  const setSecret = useSetSourceSecret();
  const act = useAct();
  const id = useId();
  const [draft, setDraft] = useState<Draft>(() => (source ? draftOf(source) : EMPTY_DRAFT));
  const set = <K extends keyof Draft>(key: K, value: Draft[K]) => setDraft((d) => ({ ...d, [key]: value }));
  const sentry = draft.kind === "sentry";
  const pending = add.isPending || update.isPending || setSecret.isPending;

  const submit = async (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const body = bodyOf(draft);
    if (source) {
      if (await act(() => update.mutateAsync(body), `Источник ${body.name} сохранён`)) onDone();
      return;
    }
    if (!(await act(() => add.mutateAsync(body), `Источник ${body.name} добавлен`))) return;
    // The secret is a second request. If it fails, the source stays and its row offers the secret again.
    if (draft.auth !== "none" && draft.secret.trim()) {
      await act(() => setSecret.mutateAsync({ name: body.name, value: draft.secret }), `Секрет ${body.name} сохранён`);
    }
    onDone();
  };

  return (
    <form onSubmit={(e) => void submit(e)} aria-label={editing ? `Изменить источник ${source.name}` : "Параметры источника"}>
      <div className="st-row">
        <label className="lbl" htmlFor={`${id}-name`}>
          <b>Имя</b>
          <span>Строчные латинские буквы, цифры, «-» и «_». Наблюдения ссылаются на источник по имени{editing ? "; изменить его нельзя" : ""}.</span>
        </label>
        <div className="ctl">
          <input
            id={`${id}-name`}
            className="st-input mono"
            value={draft.name}
            onChange={(e) => set("name", e.target.value)}
            pattern={NAME_PATTERN}
            maxLength={32}
            required
            disabled={editing}
            spellCheck={false}
            placeholder="prod-metrics"
          />
        </div>
      </div>

      <div className="st-row">
        <label className="lbl" htmlFor={`${id}-kind`}>
          <b>Тип</b>
          <span>Какой сервер опрашивает источник.</span>
        </label>
        <div className="ctl">
          <select id={`${id}-kind`} value={draft.kind} onChange={(e) => set("kind", e.target.value as SourceKind)}>
            {SOURCE_KINDS.map((kind) => (
              <option key={kind} value={kind}>
                {KIND_LABEL[kind]}
              </option>
            ))}
          </select>
        </div>
      </div>

      <div className="st-row">
        <label className="lbl" htmlFor={`${id}-url`}>
          <b>Адрес сервера</b>
          <span>Например https://prometheus.example.com. Без пути, логина, пароля и параметров в адресе.</span>
        </label>
        <div className="ctl">
          <input
            id={`${id}-url`}
            className="st-input mono"
            type="url"
            value={draft.url}
            onChange={(e) => set("url", e.target.value)}
            placeholder="https://prometheus.example.com"
            required
            spellCheck={false}
          />
        </div>
      </div>

      <div className="st-row">
        <label className="lbl" htmlFor={`${id}-auth`}>
          <b>Авторизация</b>
          <span>Как сервер входит в источник.</span>
        </label>
        <div className="ctl">
          <select id={`${id}-auth`} value={draft.auth} onChange={(e) => set("auth", e.target.value as SourceAuth)}>
            {SOURCE_AUTHS.map((auth) => (
              <option key={auth} value={auth}>
                {AUTH_LABEL[auth]}
              </option>
            ))}
          </select>
        </div>
      </div>

      {sentry && (
        <>
          <div className="st-row">
            <label className="lbl" htmlFor={`${id}-org`}>
              <b>Организация</b>
              <span>Slug организации в Sentry.</span>
            </label>
            <div className="ctl">
              <input id={`${id}-org`} className="st-input mono" value={draft.organization} onChange={(e) => set("organization", e.target.value)} placeholder="acme" required spellCheck={false} />
            </div>
          </div>
          <div className="st-row">
            <label className="lbl" htmlFor={`${id}-sentry-project`}>
              <b>Проект Sentry</b>
              <span>Slug проекта. Пусто — все проекты организации.</span>
            </label>
            <div className="ctl">
              <input id={`${id}-sentry-project`} className="st-input mono" value={draft.sentryProject} onChange={(e) => set("sentryProject", e.target.value)} placeholder="backend" spellCheck={false} />
            </div>
          </div>
          <div className="st-row">
            <label className="lbl" htmlFor={`${id}-env`}>
              <b>Окружение</b>
              <span>Пусто — все окружения.</span>
            </label>
            <div className="ctl">
              <input id={`${id}-env`} className="st-input mono" value={draft.environment} onChange={(e) => set("environment", e.target.value)} placeholder="production" spellCheck={false} />
            </div>
          </div>
          <div className="st-row">
            <label className="lbl" htmlFor={`${id}-field`}>
              <b>Поле</b>
              <span>Значение, которое берётся из ответа. По умолчанию count().</span>
            </label>
            <div className="ctl">
              <input id={`${id}-field`} className="st-input mono" value={draft.field} onChange={(e) => set("field", e.target.value)} placeholder="count()" spellCheck={false} />
            </div>
          </div>
          <div className="st-row">
            <label className="lbl" htmlFor={`${id}-dataset`}>
              <b>Набор данных</b>
              <span>По умолчанию errors.</span>
            </label>
            <div className="ctl">
              <input id={`${id}-dataset`} className="st-input mono" value={draft.dataset} onChange={(e) => set("dataset", e.target.value)} placeholder="errors" spellCheck={false} />
            </div>
          </div>
        </>
      )}

      <div className="st-row">
        <label className="lbl" htmlFor={`${id}-timeout`}>
          <b>Таймаут, сек</b>
          <span>От 1 до 120. По умолчанию 30.</span>
        </label>
        <div className="ctl">
          <input
            id={`${id}-timeout`}
            className="st-input"
            type="number"
            min={1}
            max={120}
            step={1}
            inputMode="numeric"
            value={draft.timeoutSecs}
            onChange={(e) => set("timeoutSecs", e.target.value)}
            placeholder="30"
          />
        </div>
      </div>

      {!editing && draft.auth !== "none" && (
        <div className="st-row">
          <label className="lbl" htmlFor={`${id}-secret`}>
            <b>Секрет</b>
            <span>{draft.auth === "basic" ? "Логин и пароль через двоеточие." : "Токен доступа."} После сохранения не показывается.</span>
          </label>
          <div className="ctl">
            <input
              id={`${id}-secret`}
              className="st-input mono"
              type="password"
              autoComplete="new-password"
              value={draft.secret}
              onChange={(e) => set("secret", e.target.value)}
              placeholder={draft.auth === "basic" ? "логин:пароль" : "Токен"}
              spellCheck={false}
            />
          </div>
        </div>
      )}

      <div className="st-foot">
        <span className="grow" />
        <button type="button" className="btn ghost" disabled={pending} onClick={onDone}>
          Отменить
        </button>
        <button className="btn primary" disabled={pending || !draft.name.trim() || !draft.url.trim()}>
          {pending ? "Сохраняю…" : editing ? "Сохранить" : "Добавить"}
        </button>
      </div>
    </form>
  );
}

/**
 * The secret of a source: a password field, cleared after each save, that never shows the value,
 * only its hint (§15). A source without authorization takes no secret.
 */
function SecretRow({ source }: { source: SourceView }) {
  const setSecret = useSetSourceSecret();
  const clear = useClearSourceSecret();
  const act = useAct();
  const id = useId();
  const [value, setValue] = useState("");
  const usable = source.auth !== "none";
  const busy = setSecret.isPending || clear.isPending;
  const status = source.secret ? `Задан: ${source.secret.hint}. Новое значение заменит его.` : "Не задан.";

  return (
    <form
      className="st-row"
      onSubmit={(e) => {
        e.preventDefault();
        if (!usable || !value.trim()) return;
        void act(() => setSecret.mutateAsync({ name: source.name, value }), `Секрет ${source.name} сохранён`).then((ok) => ok && setValue(""));
      }}
    >
      <label className="lbl" htmlFor={`${id}-value`}>
        <b>Секрет</b>
        <span>{usable ? (source.auth === "basic" ? "Логин и пароль через двоеточие." : "Токен доступа.") : "Для входа без авторизации секрет не нужен."}</span>
      </label>
      <div className="ctl">
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center" }}>
          <input
            id={`${id}-value`}
            className="st-input mono"
            type="password"
            autoComplete="new-password"
            spellCheck={false}
            disabled={!usable || busy}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            placeholder={source.auth === "basic" ? "логин:пароль" : "Токен"}
            style={{ flex: 1, minWidth: 180 }}
          />
          <button className="btn sm" disabled={!usable || busy || !value.trim()}>
            {setSecret.isPending ? "Сохраняю…" : source.secret ? "Заменить" : "Сохранить"}
          </button>
          {source.secret && (
            <button type="button" className="btn sm ghost" disabled={busy} onClick={() => void act(() => clear.mutateAsync(source.name), `Секрет ${source.name} удалён`)}>
              Удалить
            </button>
          )}
        </div>
        <span className="muted" style={{ fontSize: 12, lineHeight: 1.5 }}>
          {usable ? status : ""}
        </span>
      </div>
    </form>
  );
}

const DEFAULT_RANGE = "1h";

/**
 * A test query against the source over a window ending now. The value or the source's error
 * shows in the row; nothing is stored (§15).
 */
function TestRow({ source }: { source: SourceView }) {
  const check = useTestSource();
  const id = useId();
  const [query, setQuery] = useState("");
  const [range, setRange] = useState("");
  const [result, setResult] = useState<SourceTestView>();
  const [failure, setFailure] = useState<string>();

  const run = async (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!query.trim()) return;
    setResult(undefined);
    setFailure(undefined);
    try {
      setResult(await check.mutateAsync({ name: source.name, query: query.trim(), window: range.trim() || undefined }));
    } catch (err) {
      setFailure(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <form className="st-row" onSubmit={(e) => void run(e)}>
      <label className="lbl" htmlFor={`${id}-query`}>
        <b>Проверка запроса</b>
        <span>
          Что источник отдаёт за последнее окно. В Prometheus <code>$__range</code> заменяется длиной окна.
        </span>
      </label>
      <div className="ctl">
        <textarea
          id={`${id}-query`}
          className="mono"
          rows={2}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={QUERY_PLACEHOLDER[source.kind]}
          spellCheck={false}
        />
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center" }}>
          <label htmlFor={`${id}-range`} className="muted">
            Окно
          </label>
          <input
            id={`${id}-range`}
            className="st-input mono"
            value={range}
            onChange={(e) => setRange(e.target.value)}
            placeholder={DEFAULT_RANGE}
            spellCheck={false}
            style={{ width: 96, maxWidth: 96 }}
          />
          <button className="btn sm" disabled={check.isPending || !query.trim()}>
            {check.isPending ? "Проверяю…" : "Проверить"}
          </button>
        </div>
        {result && (
          <p role="status" style={{ margin: 0, fontSize: 13 }}>
            Значение <b className="mono">{result.value}</b>
            {result.samples !== undefined && <> · выборка {result.samples}</>}
            <br />
            <span className="muted" style={{ fontSize: 12 }}>
              Отправлено: <code>{result.query}</code>
            </span>
          </p>
        )}
        {failure && (
          <div role="alert" className="st-note">
            {failure}
          </div>
        )}
      </div>
    </form>
  );
}
