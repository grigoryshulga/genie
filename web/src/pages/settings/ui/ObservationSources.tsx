// The observation sources of the project (docs/platform/observation-plane.md §13.2, §15, §17):
// the servers that observation cases read their measurements from. Only project administrators
// reach the list; the server refuses everyone else, so other members see a notice instead.
// A secret is entered once, never shown again, and only its hint is listed. The rules of a draft,
// its request body and the shape of a secret live in entities/observation/model.ts.

import { type FormEvent, useId, useState } from "react";
import {
  OBSERVATION_TEXT,
  SOURCE_AUTH_LABEL,
  SOURCE_KIND_LABEL,
  SOURCE_NAME_PATTERN,
  type SourceDraft,
  emptySourceDraft,
  secretProblem,
  sourceBody,
  sourceDraft,
  useAddSource,
  useClearSourceSecret,
  useObservationSources,
  useRemoveSource,
  useSetSourceSecret,
  useTestSource,
  useUpdateSource,
} from "@/entities/observation";
import type { SourceAuth } from "@/shared/api/generated/SourceAuth.ts";
import type { SourceKind } from "@/shared/api/generated/SourceKind.ts";
import type { SourceTestView } from "@/shared/api/generated/SourceTestView.ts";
import type { SourceView } from "@/shared/api/generated/SourceView.ts";
import { ConfirmDialog, Icon, useAct, useToast } from "@/shared/ui";

const SOURCE_KINDS: SourceKind[] = ["prometheus", "sentry"];
const SOURCE_AUTHS: SourceAuth[] = ["none", "bearer", "basic"];

const URL_HINT =
  "Например https://prometheus.example.com или http://mimir:8080/prometheus. Без логина, пароля, параметров запроса и «#» в адресе.";
const URL_PLACEHOLDER: Record<SourceKind, string> = {
  prometheus: "https://prometheus.example.com",
  sentry: "https://sentry.io",
};
/** What a secret is for, per authorization kind. A source without authorization takes none. */
const SECRET_HINT: Record<SourceAuth, string> = {
  none: "Для входа без авторизации секрет не нужен.",
  bearer: "Токен доступа. После сохранения не показывается.",
  basic: "Логин и пароль через двоеточие. После сохранения не показывается.",
};
const SECRET_PLACEHOLDER: Record<SourceAuth, string> = {
  none: "",
  bearer: "Токен",
  basic: "логин:пароль",
};
const QUERY_PLACEHOLDER: Record<SourceKind, string> = {
  prometheus: "sum(rate(http_requests_total[$__range]))",
  sentry: "level:error",
};
const DEFAULT_RANGE = "1h";

/** The text of a failed request, for a toast or an inline note. */
function messageOf(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function ObservationSources({ admin }: { admin: boolean }) {
  const sources = useObservationSources(admin);
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<string>();
  const list = sources.data ?? [];

  return (
    <div className="st-body">
      <div className="st-head-row">
        <div className="st-head">
          <h2>{OBSERVATION_TEXT.section}</h2>
          <p>Источники — серверы метрик и ошибок, из которых наблюдения берут измерения. Секреты хранятся на сервере зашифрованными и после сохранения не показываются.</p>
        </div>
        {admin && !adding && (
          <button type="button" className="btn primary" onClick={() => setAdding(true)}>
            <Icon.plus size={14} />
            Добавить источник
          </button>
        )}
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
      <div className="st-card-head st-head-line">
        <div className="st-head-main">
          <h3 id={titleId} className="mono">
            {source.name}
          </h3>
          <p>
            {SOURCE_KIND_LABEL[source.kind]} · <span className="mono">{source.url}</span>
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
            <dd>{SOURCE_AUTH_LABEL[source.auth]}</dd>
            <dt>Секрет</dt>
            <dd>
              {source.secret ? (
                <>
                  Задан: <span className="mono">{source.secret.hint}</span>
                </>
              ) : (
                "Не задан"
              )}
            </dd>
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
          title={`Удалить источник ${source.name}?`}
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
          Секрет удалится вместе с ним. Наблюдения, которые его используют, перестанут получать измерения: их сбор будет записывать ошибку. Вернуть источник можно, создав
          его заново с тем же именем и снова задав секрет.
        </ConfirmDialog>
      )}
    </section>
  );
}

/** The form of a new source (no `source`) or of a stored one, whose name cannot change. */
function SourceForm({ source, onDone }: { source?: SourceView; onDone: () => void }) {
  const editing = source !== undefined;
  const add = useAddSource();
  const update = useUpdateSource();
  const setSecret = useSetSourceSecret();
  const act = useAct();
  const toast = useToast();
  const id = useId();
  const [draft, setDraft] = useState<SourceDraft>(() => (source ? sourceDraft(source) : emptySourceDraft()));
  const set = <K extends keyof SourceDraft>(key: K, value: SourceDraft[K]) => setDraft((d) => ({ ...d, [key]: value }));
  const sentry = draft.kind === "sentry";
  // A new source with an authorization kind takes its secret in the same form; it is sent after the source exists.
  const needsSecret = !editing && draft.auth !== "none";
  const secretMissing = needsSecret ? secretProblem(draft.auth, draft.secret) : undefined;
  const pending = add.isPending || update.isPending || setSecret.isPending;

  const submit = async (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const body = sourceBody(draft);
    if (source) {
      if (await act(() => update.mutateAsync(body), `Источник ${body.name} сохранён`)) onDone();
      return;
    }
    try {
      await add.mutateAsync(body);
    } catch (err) {
      toast(messageOf(err), "error");
      return;
    }
    if (needsSecret) {
      try {
        await setSecret.mutateAsync({ name: body.name, value: draft.secret });
      } catch (err) {
        setSecret.reset();
        toast(`Источник ${body.name} добавлен, но секрет не сохранён: ${messageOf(err)}. Задайте его в строке «Секрет».`, "error");
        onDone();
        return;
      }
      setSecret.reset();
    }
    toast(`Источник ${body.name} добавлен`);
    onDone();
  };

  return (
    <form onSubmit={(e) => void submit(e)} aria-label={source ? `Изменить источник ${source.name}` : "Параметры источника"}>
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
            pattern={SOURCE_NAME_PATTERN}
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
                {SOURCE_KIND_LABEL[kind]}
              </option>
            ))}
          </select>
        </div>
      </div>

      <div className="st-row">
        <label className="lbl" htmlFor={`${id}-url`}>
          <b>Адрес сервера</b>
          <span>{URL_HINT}</span>
        </label>
        <div className="ctl">
          <input
            id={`${id}-url`}
            className="st-input mono"
            type="url"
            value={draft.url}
            onChange={(e) => set("url", e.target.value)}
            placeholder={URL_PLACEHOLDER[draft.kind]}
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
                {SOURCE_AUTH_LABEL[auth]}
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

      {needsSecret && (
        <div className="st-row">
          <label className="lbl" htmlFor={`${id}-secret`}>
            <b>Секрет</b>
            <span>{SECRET_HINT[draft.auth]}</span>
          </label>
          <div className="ctl">
            <input
              id={`${id}-secret`}
              className="st-input mono"
              type="password"
              autoComplete="new-password"
              value={draft.secret}
              onChange={(e) => set("secret", e.target.value)}
              placeholder={SECRET_PLACEHOLDER[draft.auth]}
              spellCheck={false}
            />
            {draft.secret !== "" && secretMissing && <div className="st-note">{secretMissing}</div>}
          </div>
        </div>
      )}

      <div className="st-foot">
        <span className="grow" />
        <button type="button" className="btn ghost" disabled={pending} onClick={onDone}>
          Отменить
        </button>
        <button className="btn primary" disabled={pending || !draft.name.trim() || !draft.url.trim() || secretMissing !== undefined}>
          {pending ? "Сохраняю…" : source ? "Сохранить" : "Добавить"}
        </button>
      </div>
    </form>
  );
}

/**
 * The secret of a stored source: a password field, cleared after each successful save, that never
 * shows the value, only its hint (§15). A source without authorization takes no secret.
 */
function SecretRow({ source }: { source: SourceView }) {
  const setSecret = useSetSourceSecret();
  const clear = useClearSourceSecret();
  const act = useAct();
  const id = useId();
  const [value, setValue] = useState("");
  const usable = source.auth !== "none";
  const problem = secretProblem(source.auth, value);
  const busy = setSecret.isPending || clear.isPending;
  const status = source.secret
    ? `Задан: ${source.secret.hint}.${usable ? " Новое значение заменит его." : " Сейчас не используется."}`
    : "Не задан.";

  const save = async () => {
    if (!usable || problem) return;
    const ok = await act(() => setSecret.mutateAsync({ name: source.name, value }), `Секрет ${source.name} сохранён`);
    setSecret.reset();
    if (ok) setValue("");
  };

  return (
    <form
      className="st-row"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <label className="lbl" htmlFor={`${id}-value`}>
        <b>Секрет</b>
        <span>{SECRET_HINT[source.auth]}</span>
      </label>
      <div className="ctl">
        <div className="st-inline">
          <input
            id={`${id}-value`}
            className="st-input mono"
            type="password"
            autoComplete="new-password"
            spellCheck={false}
            disabled={!usable || busy}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            placeholder={SECRET_PLACEHOLDER[source.auth]}
          />
          <button className="btn sm" disabled={!usable || busy || problem !== undefined}>
            {setSecret.isPending ? "Сохраняю…" : source.secret ? "Заменить" : "Сохранить"}
          </button>
          {source.secret && (
            <button type="button" className="btn sm ghost" disabled={busy} onClick={() => void act(() => clear.mutateAsync(source.name), `Секрет ${source.name} удалён`)}>
              Удалить
            </button>
          )}
        </div>
        {value !== "" && problem && <div className="st-note">{problem}</div>}
        <span className="st-hint">{status}</span>
      </div>
    </form>
  );
}

/**
 * A test query against the source over a window ending now. The value, or the source's error,
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
      setFailure(messageOf(err));
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
        <div className="st-inline">
          <label htmlFor={`${id}-range`} className="st-hint">
            Окно
          </label>
          <input id={`${id}-range`} className="st-input mono st-input-short" value={range} onChange={(e) => setRange(e.target.value)} placeholder={DEFAULT_RANGE} spellCheck={false} />
          <button className="btn sm" disabled={check.isPending || !query.trim()}>
            {check.isPending ? "Проверяю…" : "Проверить"}
          </button>
        </div>
        {result && (
          <p role="status" className="st-result">
            Значение <b className="mono">{result.value}</b>
            {result.samples !== undefined && <> · выборка {result.samples}</>}
            <br />
            <span className="st-hint">
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
