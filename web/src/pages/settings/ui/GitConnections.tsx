// Accounts connected through the GitHub App, and the picker that attaches their repositories
// to the project. A connection belongs to the person who made it; the project reaches only the
// repositories attached here, so the picker is where access is granted.

import { useState } from "react";
import {
  type ConnectionHealth,
  type GitConnection,
  HEALTH_NAME,
  PRESETS,
  type ProviderRepo,
  repoNameFrom,
  useAddRepo,
  useAuthorizeConnection,
  useCheckConnection,
  useConnectGitHub,
  useConnectionRepos,
  useDisconnect,
  useGitConnections,
  useReassignConnection,
  useRepos,
} from "@/entities/repo";
import { ApiError } from "@/shared/api";
import { ConfirmDialog, Icon, useAct } from "@/shared/ui";

const DEFAULT_POLICY = PRESETS.find((p) => p.id === "pr-human")!.policy;

const LEVEL: Record<ConnectionHealth, "ok" | "warn" | "fail"> = {
  ok: "ok",
  needs_attention: "warn",
  permission_missing: "warn",
  suspended: "fail",
  revoked: "fail",
  owner_removed: "fail",
};

/** Go to GitHub (installation page or confirmation) and come back to the settings. */
function leave(url: string) {
  window.location.assign(url);
}

export function ConnectionsCard({ admin, picking, failed }: { admin: boolean; picking?: string; failed?: boolean }) {
  const info = useGitConnections();
  const connect = useConnectGitHub();
  const act = useAct();
  const [open, setOpen] = useState<string | undefined>(picking);
  if (!admin) return null;
  const data = info.data;
  const list = data?.connections ?? [];
  const opened = list.find((c) => c.id === open);

  return (
    <section className="st-card gc" aria-label="Подключённые аккаунты">
      <div className="gc-head">
        <div className="gc-title">
          <b>Подключённые аккаунты</b>
          <span className="rp-hint">Доступ сервера к репозиториям через GitHub App: токены выдаются на каждую операцию и нигде не хранятся.</span>
        </div>
        {data?.github.configured && data.canConnect && (
          <button
            type="button"
            className="g-btn secondary"
            disabled={connect.isPending}
            onClick={() => void act(async () => leave((await connect.mutateAsync()).url))}
          >
            <Icon.plus size={14} />
            {list.length ? "Подключить ещё" : "Подключить GitHub"}
          </button>
        )}
      </div>

      {failed && (
        <p className="gc-note fail" role="alert">
          <i />
          GitHub не подтвердил подключение: начните заново. Установка должна принадлежать вашему аккаунту.
        </p>
      )}
      {data && !data.github.configured && (
        <p className="gc-note">
          На сервере не настроено приложение GitHub (<span className="g-mono">gitApps.github</span> в config.json). Пока подключить аккаунт нельзя — репозиторий можно добавить по ссылке и токену.
        </p>
      )}
      {data?.github.configured && !data.canConnect && list.length === 0 && <p className="gc-note">Подключать аккаунты могут администраторы проектов.</p>}

      {list.length > 0 && (
        <ul className="gc-list">
          {list.map((c) => (
            <Connection key={c.id} c={c} onPick={() => setOpen(open === c.id ? undefined : c.id)} picking={open === c.id} />
          ))}
        </ul>
      )}
      {opened && <RepoPicker connection={opened} onClose={() => setOpen(undefined)} />}
    </section>
  );
}

function Connection({ c, onPick, picking }: { c: GitConnection; onPick: () => void; picking: boolean }) {
  const check = useCheckConnection();
  const disconnect = useDisconnect();
  const reassign = useReassignConnection();
  const act = useAct();
  const [confirm, setConfirm] = useState(false);
  const [login, setLogin] = useState("");
  const level = LEVEL[c.health];
  const usable = !c.blocked;

  return (
    <li className="gc-item">
      <div className="gc-row">
        <span className="rp-ic" aria-hidden>
          <Icon.branch size={16} />
        </span>
        <div className="rp-id">
          <span className="rp-title">
            <b>{c.accountLogin}</b>
            <span className={`rp-status ${level}`} title={c.statusNote || undefined}>
              <i />
              {HEALTH_NAME[c.health]}
            </span>
          </span>
          <span className="rp-meta">
            <span>GitHub</span>
            <span aria-hidden>·</span>
            <span>{c.selection === "all" ? "все репозитории аккаунта" : "выбранные репозитории"}</span>
            <span aria-hidden>·</span>
            <span>{c.ownerLogin ? `подключил ${c.ownerLogin}` : "владельца нет"}</span>
            <span aria-hidden>·</span>
            <span>в проектах: {c.repos}</span>
            {c.manageUrl && (
              <a href={c.manageUrl} target="_blank" rel="noreferrer" title="Настройки установки на GitHub">
                на GitHub
                <Icon.external size={11} />
              </a>
            )}
          </span>
        </div>
        {usable && (
          <button type="button" className="g-btn secondary sm" aria-expanded={picking} onClick={onPick}>
            Выбрать репозитории
          </button>
        )}
        <button
          type="button"
          className="g-btn ghost sm"
          disabled={check.isPending}
          onClick={() => void act(() => check.mutateAsync(c.id), `${c.accountLogin}: состояние обновлено`)}
        >
          {check.isPending ? "Проверяю…" : "Проверить"}
        </button>
        {c.health !== "revoked" && (
          <button type="button" className="g-btn ghost sm gc-off" onClick={() => setConfirm(true)}>
            Отключить
          </button>
        )}
      </div>

      {c.blocked && (
        <p className="gc-note fail" role="alert">
          <i />
          {c.blocked}
        </p>
      )}
      {c.health === "owner_removed" && (
        <form
          className="gc-reassign"
          onSubmit={(e) => {
            e.preventDefault();
            if (!login.trim()) return;
            void act(() => reassign.mutateAsync({ id: c.id, login: login.trim() }), `${c.accountLogin}: передано ${login.trim()}`).then((ok) => ok && setLogin(""));
          }}
        >
          <label htmlFor={`gc-login-${c.id}`}>Передать подключение</label>
          <input id={`gc-login-${c.id}`} className="st-input" placeholder="логин нового владельца" value={login} onChange={(e) => setLogin(e.target.value)} autoComplete="off" />
          <button className="g-btn secondary sm" disabled={reassign.isPending || !login.trim()}>
            Передать
          </button>
          <span className="rp-hint">Доступно администратору сервера. Пока подключение без владельца, операции с ним остановлены.</span>
        </form>
      )}

      {confirm && (
        <ConfirmDialog
          title={`Отключить ${c.accountLogin}?`}
          confirmLabel="Отключить"
          danger
          busy={disconnect.isPending}
          onClose={() => setConfirm(false)}
          onConfirm={() => void act(() => disconnect.mutateAsync(c.id), `${c.accountLogin} отключён`).then(() => setConfirm(false))}
        >
          Сервер сразу перестанет работать с репозиториями этого аккаунта: новые поставки и обновления остановятся. Репозитории в проектах, задачи и ссылки на запросы на слияние
          останутся. Приложение на GitHub не удаляется — это можно сделать в его настройках; повторное подключение вернёт доступ.
        </ConfirmDialog>
      )}
    </li>
  );
}

/** The repositories the person and the installation both reach; one click attaches one to the project. */
function RepoPicker({ connection, onClose }: { connection: GitConnection; onClose: () => void }) {
  const list = useConnectionRepos(connection.id);
  const authorize = useAuthorizeConnection();
  const repos = useRepos();
  const act = useAct();
  const [q, setQ] = useState("");
  const needsAuthorization = list.error instanceof ApiError && list.error.status === 409 && list.error.message.startsWith("authorize");
  const all = list.data?.repositories ?? [];
  const shown = all.filter((r) => r.fullName.toLowerCase().includes(q.trim().toLowerCase()));
  const attached = (repos.data ?? []).length;

  return (
    <div className="gc-picker" role="region" aria-label={`Репозитории ${connection.accountLogin}`}>
      <div className="rp-add-head">
        <h3>Репозитории {connection.accountLogin}</h3>
        <button type="button" className="g-icon-btn" aria-label="Закрыть" onClick={onClose}>
          <Icon.close size={14} />
        </button>
      </div>
      {needsAuthorization ? (
        <div className="gc-auth">
          <p>
            Чтобы показать только то, что доступно и вам, и приложению, GitHub нужно подтвердить доступ. Токен не сохраняется: он живёт в памяти сервера не дольше 15 минут.
          </p>
          <button type="button" className="g-btn secondary" disabled={authorize.isPending} onClick={() => void act(async () => leave((await authorize.mutateAsync(connection.id)).url))}>
            Подтвердить на GitHub
          </button>
        </div>
      ) : list.isPending ? (
        <p className="gc-note">Загружаю…</p>
      ) : list.error ? (
        <p className="gc-note fail" role="alert">
          <i />
          {list.error.message}
        </p>
      ) : all.length === 0 ? (
        <p className="gc-note">
          У приложения нет ни одного репозитория. Добавьте их в настройках установки на GitHub{connection.manageUrl ? "" : ""}, затем откройте список снова.
        </p>
      ) : (
        <>
          <div className="gc-search">
            <input className="st-input" placeholder="Найти репозиторий" aria-label="Найти репозиторий" value={q} onChange={(e) => setQ(e.target.value)} autoComplete="off" />
            <span className="rp-hint">
              {shown.length} из {all.length}
            </span>
          </div>
          <ul className="gc-repos">
            {shown.map((r) => (
              <PickRow key={r.id} connection={connection} repo={r} first={attached === 0} />
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

function PickRow({ connection, repo, first }: { connection: GitConnection; repo: ProviderRepo; first: boolean }) {
  const add = useAddRepo();
  const act = useAct();
  const [name, setName] = useState(repoNameFrom(repo.fullName));
  const valid = /^[a-z0-9][a-z0-9_-]*$/.test(name);
  return (
    <li className="gc-repo">
      <div className="gc-repo-id">
        <span className="g-mono">{repo.fullName}</span>
        <span className="rp-hint">
          {repo.private ? "приватный" : "публичный"} · ветка <span className="g-mono">{repo.defaultBranch}</span>
          {!repo.canPush && " · у вас нет права записи"}
        </span>
      </div>
      {repo.boundAs ? (
        <span className="rp-found">
          <Icon.check size={14} />
          в проекте как <b>{repo.boundAs}</b>
        </span>
      ) : (
        <form
          className="rp-inline"
          onSubmit={(e) => {
            e.preventDefault();
            if (!valid) return;
            void act(
              async () => {
                const r = await add.mutateAsync({
                  name,
                  connection: connection.id,
                  providerRepoId: repo.id,
                  // The first repository sits at the root of the workspace, the next ones in a folder of their own.
                  mount: first ? "." : name,
                  access: "write",
                  policy: DEFAULT_POLICY,
                });
                if (r.warning) throw new Error(`Добавлен, но хостинг не ответил: ${r.warning}`);
              },
              `${name} добавлен в проект`,
            );
          }}
        >
          <input
            className="st-input g-mono rp-short"
            aria-label={`Имя ${repo.fullName} в проекте`}
            value={name}
            onChange={(e) => setName(e.target.value)}
            pattern="[a-z0-9][a-z0-9_\-]*"
            required
          />
          <button className="g-btn secondary sm" disabled={add.isPending || !valid}>
            {add.isPending ? "Добавляю…" : "Добавить"}
          </button>
        </form>
      )}
    </li>
  );
}
