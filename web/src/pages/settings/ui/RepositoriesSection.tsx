// The project's repositories: where they live, where they sit in the workspace, what agents
// may do in each. Everybody in the project sees the list; admins attach, change and check them.
// The page reads from the top down: what the project has, then what agents may do in each
// repository; rare things (the folder, the rule as JSON, a probe push) wait under a disclosure or a menu.

import { useState } from "react";
import {
  type CheckLine,
  type GitHostInfo,
  parseRepoLink,
  type ProjectRepo,
  PRESETS,
  presetName,
  presetOf,
  type RepoPolicy,
  repoNameFrom,
  useAddRepo,
  useCheckRepo,
  useGitHosts,
  usePatchRepo,
  useRemoveRepo,
  useRepos,
  useSyncRepo,
} from "@/entities/repo";
import { ConfirmDialog, Icon, Menu, useAct } from "@/shared/ui";
import { SettingsLoading } from "@/widgets/settings-shell";

type Access = "read" | "write";

/** The rules offered once agents may write: "read only" is the other side of the switch. */
const WRITE_PRESETS = PRESETS.filter((p) => p.id !== "read");
const READ_POLICY = PRESETS.find((p) => p.id === "read")!.policy;
const DEFAULT_PRESET = "pr-human";

const HOST_HINT =
  "Доступ к хостингу выполняет сервер. Хостинги настраиваются в git.json.";

export function RepositoriesHead({ admin, onAdd }: { admin: boolean; onAdd?: () => void }) {
  return (
    <div className="rp-top">
      <div className="st-head">
        <h2>Репозитории</h2>
        <p className="rp-lead">
          Хостинги
          <span className="rp-info" title={HOST_HINT} aria-label={HOST_HINT} role="img">
            <Icon.info size={14} />
          </span>
        </p>
      </div>
      {admin && onAdd && (
        <button type="button" className="g-btn secondary" onClick={onAdd}>
          <Icon.plus size={14} />
          Добавить репозиторий
        </button>
      )}
    </div>
  );
}

export function RepositoriesSection({ admin }: { admin: boolean }) {
  const repos = useRepos();
  const hosts = useGitHosts(admin);
  const list = repos.data ?? [];
  const [adding, setAdding] = useState(false);
  return (
    <>
      <RepositoriesHead admin={admin} onAdd={adding ? undefined : () => setAdding(true)} />
      {admin && adding && <AddRepo hosts={hosts.data?.hosts ?? []} problems={hosts.data?.errors ?? []} loaded={!hosts.isPending} onClose={() => setAdding(false)} />}
      {list.length > 0 ? (
        <ul className="rp-list" aria-label="Репозитории проекта">
          {list.map((r) => (
            <Repo key={`${r.name}:${r.mount}:${r.access}:${JSON.stringify(r.policy)}`} repo={r} admin={admin} />
          ))}
        </ul>
      ) : (
        !adding && (
          <div className="st-card rp-empty">
            {repos.isPending ? (
              <SettingsLoading />
            ) : (
              <>
                <b>Репозиториев пока нет</b>
                <p className="g-muted">Без них результат задач — артефакты и страницы документации. Добавьте репозиторий, чтобы агенты работали с кодом.</p>
              </>
            )}
          </div>
        )
      )}
    </>
  );
}

/** Two options side by side: what agents may do in a repository. */
function AccessSwitch({ label, value, onChange, disabled }: { label: string; value: Access; onChange: (v: Access) => void; disabled?: boolean }) {
  return (
    <div className="rp-seg" role="radiogroup" aria-label={label}>
      {(
        [
          ["read", "Только читать"],
          ["write", "Читать и предлагать изменения"],
        ] as const
      ).map(([v, t]) => (
        <button key={v} type="button" role="radio" aria-checked={value === v} className={value === v ? "on" : undefined} disabled={disabled} onClick={() => value !== v && onChange(v)}>
          {t}
        </button>
      ))}
    </div>
  );
}

/** The rule for a repository agents write to, with what it means under it. */
function RuleSelect({ id, value, onChange }: { id: string; value: string; onChange: (id: string) => void }) {
  const p = PRESETS.find((x) => x.id === value);
  return (
    <div className="rp-rule">
      <select id={id} value={value} onChange={(e) => onChange(e.target.value)}>
        {value === "custom" && <option value="custom">{presetName("custom")}</option>}
        {WRITE_PRESETS.map((x) => (
          <option key={x.id} value={x.id}>
            {x.name}
          </option>
        ))}
      </select>
      <span className={p?.risky ? "rp-hint warn" : "rp-hint"}>{p?.hint ?? "Правило задано вручную, в JSON — см. «Дополнительно»."}</span>
    </div>
  );
}

function checkLevel(lines: CheckLine[]): CheckLine["level"] {
  return lines.some((l) => l.level === "fail") ? "fail" : lines.some((l) => l.level === "warn") ? "warn" : "ok";
}

const LEVEL_NAME: Record<CheckLine["level"], string> = { ok: "доступен", warn: "есть замечания", fail: "нет доступа" };

function Repo({ repo, admin }: { repo: ProjectRepo; admin: boolean }) {
  const patch = usePatchRepo();
  const sync = useSyncRepo();
  const remove = useRemoveRepo();
  const check = useCheckRepo();
  const act = useAct();
  const [removing, setRemoving] = useState(false);
  const [report, setReport] = useState<CheckLine[]>();
  const [json, setJson] = useState<string>();
  const [mount, setMount] = useState(repo.mount === "." ? "" : repo.mount);
  const [token, setToken] = useState("");
  const preset = presetOf(repo.policy);
  const access: Access = repo.access === "read" || preset === "read" ? "read" : "write";
  const save = (p: { access?: Access; policy?: RepoPolicy; mount?: string; token?: string }, ok: string) => act(() => patch.mutateAsync({ name: repo.name, patch: p }), ok);
  const runCheck = (probe: boolean) => void act(async () => setReport((await check.mutateAsync({ name: repo.name, probe })).lines));

  const noToken = !!repo.host.kind && repo.host.kind !== "plain" && !repo.token?.set;
  const hostProblems = [
    ...(repo.host.error ? [repo.host.error] : []),
    ...(repo.host.problems ?? []),
    ...(noToken ? ["у репозитория нет токена доступа — задайте его ниже"] : []),
  ];
  const status: CheckLine["level"] | undefined = hostProblems.length || !repo.policyValid ? "fail" : report ? checkLevel(report) : undefined;
  const where = `${repo.host.id} ${repo.remote}`;

  return (
    <li className="rp st-card">
      <div className="rp-head">
        <span className="rp-ic" aria-hidden>
          <Icon.branch size={16} />
        </span>
        <div className="rp-id">
          <span className="rp-title">
            <b>{repo.name}</b>
            {status && (
              <span className={`rp-status ${status}`} title={report?.map((l) => l.text).join("\n") ?? hostProblems.join("\n")}>
                <i />
                {LEVEL_NAME[status]}
              </span>
            )}
          </span>
          <span className="rp-meta">
            {repo.host.webUrl ? (
              <a href={repo.host.webUrl} target="_blank" rel="noreferrer" title="Открыть на хостинге">
                {repo.host.id} <span className="g-mono">{repo.remote}</span>
                <Icon.external size={11} />
              </a>
            ) : (
              <span title={where}>
                {repo.host.id} <span className="g-mono">{repo.remote}</span>
              </span>
            )}
            <span aria-hidden>·</span>
            <span>
              ветка <span className="g-mono">{repo.defaultBranch || "не определена"}</span>
            </span>
            {repo.mount !== "." && (
              <>
                <span aria-hidden>·</span>
                <span>
                  папка <span className="g-mono">{repo.mount}</span>
                </span>
              </>
            )}
          </span>
        </div>
        {admin && (
          <>
            <button type="button" className="g-btn secondary sm" disabled={check.isPending} onClick={() => runCheck(false)}>
              {check.isPending ? "Проверяю…" : "Проверить доступ"}
            </button>
            <Menu label={`Ещё действия с ${repo.name}`}>
              {(close) => (
                <>
                  <button type="button" role="menuitem" disabled={sync.isPending} onClick={() => (close(), void act(() => sync.mutateAsync(repo.name), `${repo.name}: ветки обновлены с хостинга`))}>
                    Обновить ветки с хостинга
                  </button>
                  <button
                    type="button"
                    role="menuitem"
                    title="Создаст и удалит временную ветку на хостинге"
                    disabled={check.isPending}
                    onClick={() => (close(), runCheck(true))}
                  >
                    Проверить с пробной отправкой
                  </button>
                  <hr />
                  <button type="button" role="menuitem" className="danger" onClick={() => (close(), setRemoving(true))}>
                    Отсоединить от проекта…
                  </button>
                </>
              )}
            </Menu>
          </>
        )}
      </div>

      {(hostProblems.length > 0 || !repo.policyValid || report) && (
        <ul className="rp-report">
          {hostProblems.map((p) => (
            <li key={p} className="fail">
              <i />
              Хостинг {repo.host.id}: {p}
            </li>
          ))}
          {!repo.policyValid && (
            <li className="fail">
              <i />
              Правило не читается: пока его не исправят, агенты не получают доступа.
            </li>
          )}
          {report?.map((l, i) => (
            <li key={i} className={l.level}>
              <i />
              {l.text}
            </li>
          ))}
        </ul>
      )}

      <div className="rp-grid">
        <span className="rp-lbl">Что можно агентам</span>
        {admin ? (
          <AccessSwitch
            label={`Что можно агентам в ${repo.name}`}
            value={access}
            disabled={patch.isPending}
            onChange={(v) =>
              void (v === "read"
                ? save({ access: "read" }, `${repo.name}: агенты только читают`)
                : save({ access: "write", ...(preset === "read" ? { policy: PRESETS.find((p) => p.id === DEFAULT_PRESET)!.policy } : {}) }, `${repo.name}: агенты предлагают изменения`))
            }
          />
        ) : (
          <span>{access === "read" ? "Только читать" : "Читать и предлагать изменения"}</span>
        )}
        {access === "write" && (
          <>
            <label className="rp-lbl top" htmlFor={`rp-rule-${repo.name}`}>
              Путь в основную ветку
            </label>
            {admin ? (
              <RuleSelect
                id={`rp-rule-${repo.name}`}
                value={preset}
                onChange={(id) => {
                  const p = PRESETS.find((x) => x.id === id);
                  if (p) void save({ policy: p.policy }, `${repo.name}: ${p.name.toLowerCase()}`);
                }}
              />
            ) : (
              <div className="rp-rule">
                <span>{presetName(preset)}</span>
                <span className="rp-hint">{PRESETS.find((p) => p.id === preset)?.hint}</span>
              </div>
            )}
          </>
        )}
      </div>

      {admin && (
        <div className="rp-grid">
          <label className="rp-lbl top" htmlFor={`rp-token-${repo.name}`}>
            Токен доступа
            <span>PAT на хостинге.</span>
          </label>
          <div className="rp-json">
            <form
              className="rp-inline"
              onSubmit={(e) => {
                e.preventDefault();
                if (!token.trim()) return;
                void act(() => patch.mutateAsync({ name: repo.name, patch: { token: token.trim() } }), `${repo.name}: токен сохранён`).then((ok) => ok && setToken(""));
              }}
            >
              <input
                id={`rp-token-${repo.name}`}
                className="st-input g-mono"
                type="password"
                autoComplete="new-password"
                spellCheck={false}
                placeholder={repo.token?.set ? "Новый токен" : "Вставьте токен"}
                value={token}
                onChange={(e) => setToken(e.target.value)}
              />
              {token.trim() && (
                <button className="g-btn secondary sm" disabled={patch.isPending}>
                  {repo.token?.set ? "Заменить" : "Сохранить"}
                </button>
              )}
              {repo.token?.set && !token && (
                <button type="button" className="g-btn ghost sm" disabled={patch.isPending} onClick={() => void save({ token: "" }, `${repo.name}: токен удалён`)}>
                  Удалить
                </button>
              )}
            </form>
            <span className={repo.token?.unreadable || noToken ? "rp-hint warn" : "rp-hint"}>
              {repo.token?.set
                ? repo.token.unreadable
                  ? "Сохранённый токен не читается (сменился ключ сервера): введите его заново."
                  : `Задан${repo.token.hint ? ` (${repo.token.hint})` : ""}. Хранится на сервере зашифрованным и нигде не показывается.`
                : "Токена нет: без него сервер не сможет работать с этим репозиторием."}
            </span>
          </div>
        </div>
      )}

      {admin && (
        <details className="rp-more" onToggle={(e) => (e.currentTarget as HTMLDetailsElement).open && json === undefined && setJson(JSON.stringify(repo.policy, null, 2))}>
          <summary>Дополнительно: папка в рабочей копии, правило в JSON</summary>
          <div className="rp-grid">
            <label className="rp-lbl top" htmlFor={`rp-mount-${repo.name}`}>
              Папка
              <span>Нужна, когда в проекте несколько репозиториев.</span>
            </label>
            <form
              className="rp-inline"
              onSubmit={(e) => {
                e.preventDefault();
                void save({ mount: mount.trim() || "." }, `${repo.name}: папка сохранена`);
              }}
            >
              <input id={`rp-mount-${repo.name}`} className="st-input g-mono" placeholder="в корне" value={mount} onChange={(e) => setMount(e.target.value)} />
              {(mount.trim() || ".") !== repo.mount && (
                <button className="g-btn secondary sm" disabled={patch.isPending}>
                  Сохранить
                </button>
              )}
            </form>
            <label className="rp-lbl top" htmlFor={`rp-json-${repo.name}`}>
              Правило в JSON
              <span>Для случаев, которых нет в списке.</span>
            </label>
            {json !== undefined && (
              <div className="rp-json">
                <textarea id={`rp-json-${repo.name}`} className="g-mono" rows={8} value={json} onChange={(e) => setJson(e.target.value)} spellCheck={false} />
                <span className="rp-hint">Поля: read, push (none, pr_only, branches, direct), branch, branches, protected, force_push, delete_branches, change_request.</span>
                <button
                  type="button"
                  className="g-btn secondary sm"
                  disabled={patch.isPending || json === JSON.stringify(repo.policy, null, 2)}
                  onClick={() => {
                    let policy: RepoPolicy;
                    try {
                      policy = JSON.parse(json) as RepoPolicy;
                    } catch (e) {
                      return void act(() => Promise.reject(new Error(`Не JSON: ${e instanceof Error ? e.message : String(e)}`)));
                    }
                    void save({ policy }, `${repo.name}: правило сохранено`);
                  }}
                >
                  Сохранить правило
                </button>
              </div>
            )}
          </div>
        </details>
      )}

      {removing && (
        <ConfirmDialog
          title="Отсоединить репозиторий?"
          confirmLabel="Отсоединить"
          danger
          busy={remove.isPending}
          onClose={() => setRemoving(false)}
          onConfirm={() => void act(() => remove.mutateAsync(repo.name), `${repo.name} отсоединён`).then(() => setRemoving(false))}
        >
          Репозиторий останется на хостинге. Новые команды его не получат; рабочие копии действующих задач остаются на диске сервера. Пока у репозитория есть незакрытая поставка
          (открытый запрос), сервер откажет.
        </ConfirmDialog>
      )}
    </li>
  );
}

function AddRepo({ hosts, problems, loaded, onClose }: { hosts: GitHostInfo[]; problems: string[]; loaded: boolean; onClose: () => void }) {
  const add = useAddRepo();
  const act = useAct();
  const [link, setLink] = useState("");
  const [pickedHost, setPickedHost] = useState("");
  const [name, setName] = useState<string>();
  const [access, setAccess] = useState<Access>("write");
  const [preset, setPreset] = useState(DEFAULT_PRESET);
  const [mount, setMount] = useState("");
  const [token, setToken] = useState("");
  const parsed = parseRepoLink(link, hosts);
  // A bare `group/repo` needs a host only when the server has more than one.
  const host = parsed.kind === "ok" ? parsed.host : parsed.kind === "path" ? (hosts.length === 1 ? hosts[0].id : pickedHost) : "";
  const remote = parsed.kind === "ok" || parsed.kind === "path" ? parsed.remote : "";
  const finalName = (name ?? repoNameFrom(remote)).trim();
  const known = !!host && !!remote;
  const hostInfo = hosts.find((h) => h.id === host);
  const ready = known && /^[a-z0-9][a-z0-9_-]*$/.test(finalName);

  if (loaded && hosts.length === 0) {
    return (
      <section className="st-card rp-add">
        <AddHead onClose={onClose} />
        <div className="st-card-body">
          <p className="st-note">
            Сервер пока не знает ни одного хостинга. Администратор сервера описывает его (адрес) в <b>git.json</b>; токен можно задать у самого репозитория; пример — в docs/platform/git-repositories.md.
          </p>
          {problems.map((p) => (
            <p key={p} className="rp-hint warn">
              git.json: {p}
            </p>
          ))}
        </div>
      </section>
    );
  }

  return (
    <form
      className="st-card rp-add"
      onSubmit={(e) => {
        e.preventDefault();
        if (!ready) return;
        const policy = access === "read" ? READ_POLICY : PRESETS.find((p) => p.id === preset)?.policy;
        void act(
          async () => {
            const r = await add.mutateAsync({ name: finalName, host, remote, mount: mount.trim() || ".", access, policy, token: token.trim() || undefined });
            if (r.warning) throw new Error(`Добавлен, но хостинг не ответил: ${r.warning}`);
          },
          `${finalName} добавлен`,
        ).then((ok) => ok && onClose());
      }}
    >
      <AddHead onClose={onClose} />

      <div className="st-row">
        <label className="lbl" htmlFor="rp-link">
          <b>Ссылка</b>
          <span>Из адресной строки браузера или из кнопки Clone.</span>
        </label>
        <div className="ctl">
          <input
            id="rp-link"
            className={`st-input rp-link ${parsed.kind === "unknown" || parsed.kind === "bad" ? "warn" : known ? "ok" : ""}`}
            placeholder={hosts[0] ? `${hosts[0].url.replace(/\/+$/, "")}/группа/репозиторий` : "https://…"}
            value={link}
            onChange={(e) => setLink(e.target.value)}
            autoComplete="off"
            spellCheck={false}
            autoFocus
          />
          {parsed.kind === "empty" && <span className="rp-hint">Подойдёт и https://…, и git@…</span>}
          {known && (
            <span className="rp-found">
              <Icon.check size={14} />
              Хостинг <b>{host}</b>
              <span aria-hidden>·</span>
              <span className="g-mono">{remote}</span>
            </span>
          )}
          {parsed.kind === "path" && hosts.length > 1 && (
            <select className="st-input" aria-label="Хостинг" value={pickedHost} onChange={(e) => setPickedHost(e.target.value)}>
              <option value="">На каком хостинге?</option>
              {hosts.map((h) => (
                <option key={h.id} value={h.id}>
                  {h.id} · {h.url}
                </option>
              ))}
            </select>
          )}
          {parsed.kind === "unknown" && (
            <span className="rp-hint warn">
              Сервер не знает хостинг {parsed.hostname}. Подключены: {hosts.map((h) => h.id).join(", ")}. Новый хостинг добавляет администратор сервера.
            </span>
          )}
          {parsed.kind === "bad" && <span className="rp-hint warn">Не похоже на ссылку на репозиторий: нужен путь вида группа/репозиторий.</span>}
          {problems.map((p) => (
            <span key={p} className="rp-hint warn">
              git.json: {p}
            </span>
          ))}
        </div>
      </div>

      <fieldset className="rp-steps" disabled={!known}>
        <div className="st-row">
          <label className="lbl" htmlFor="rp-name">
            <b>Имя в проекте</b>
            <span>Так репозиторий называют задачи и агенты.</span>
          </label>
          <div className="ctl">
            <input id="rp-name" className="st-input g-mono rp-short" placeholder="api" value={name ?? repoNameFrom(remote)} onChange={(e) => setName(e.target.value)} pattern="[a-z0-9][a-z0-9_\-]*" required />
            {finalName && !/^[a-z0-9][a-z0-9_-]*$/.test(finalName) && <span className="rp-hint warn">Строчные латинские буквы, цифры, «-» и «_».</span>}
          </div>
        </div>

        <div className="st-row">
          <label className="lbl" htmlFor="rp-token-new">
            <b>Токен доступа</b>
            <span>PAT на хостинге, под которым сервер работает с этим репозиторием.</span>
          </label>
          <div className="ctl">
            <input
              id="rp-token-new"
              className="st-input g-mono"
              type="password"
              autoComplete="new-password"
              spellCheck={false}
              placeholder={hostInfo?.kind === "plain" ? "не нужен для простого git-сервера" : "glpat-… / github_pat_…"}
              value={token}
              onChange={(e) => setToken(e.target.value)}
            />
            <span className={hostInfo && hostInfo.kind !== "plain" && !token.trim() ? "rp-hint warn" : "rp-hint"}>
              Сервер хранит токен зашифрованным и нигде не показывает. Права: чтение и запись веток, запросы на слияние.
            </span>
          </div>
        </div>

        <div className="st-row">
          <div className="lbl">
            <b>Что можно агентам</b>
          </div>
          <div className="ctl">
            <AccessSwitch label="Что можно агентам" value={access} onChange={setAccess} />
            {access === "write" && (
              <>
                <label className="rp-sub" htmlFor="rp-rule-new">
                  Как изменения попадают в основную ветку
                </label>
                <RuleSelect id="rp-rule-new" value={preset} onChange={setPreset} />
              </>
            )}
          </div>
        </div>

        <details className="rp-more">
          <summary>Дополнительно: папка в рабочей копии</summary>
          <div className="rp-grid">
            <label className="rp-lbl top" htmlFor="rp-mount-new">
              Папка
              <span>Нужна, когда в проекте несколько репозиториев.</span>
            </label>
            <input id="rp-mount-new" className="st-input g-mono rp-short" placeholder="в корне" value={mount} onChange={(e) => setMount(e.target.value)} />
          </div>
        </details>
      </fieldset>

      <div className="st-foot">
        <span className="grow" />
        <button type="button" className="g-btn ghost" onClick={onClose}>
          Отменить
        </button>
        <button className="g-btn secondary" disabled={add.isPending || !ready}>
          {add.isPending ? "Добавляю…" : "Добавить"}
        </button>
      </div>
    </form>
  );
}

function AddHead({ onClose }: { onClose: () => void }) {
  return (
    <div className="rp-add-head">
      <h3>Новый репозиторий</h3>
      <button type="button" className="g-icon-btn" aria-label="Закрыть" onClick={onClose}>
        <Icon.close size={14} />
      </button>
    </div>
  );
}
