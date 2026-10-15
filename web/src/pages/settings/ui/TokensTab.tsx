// Personal tokens for the command line, scripts and one's own agent over MCP:
// how to connect, issuing a token (shown once), the list with its uses, revoking.

import { useState } from "react";
import { type UserToken, useIssueToken, useRevokeToken, useTokens } from "@/entities/project";
import { timeAgo, useTick } from "@/shared/lib";
import { ConfirmDialog, Icon, useRefreshingAct as useAct, useToast } from "@/shared/ui";

type Client = "claude" | "codex" | "cli";

const CLIENTS: { id: Client; title: string }[] = [
  { id: "claude", title: "Claude Code" },
  { id: "codex", title: "Codex" },
  { id: "cli", title: "CLI" },
];

function howTo(client: Client, origin: string, token: string): { step: string; snippet: string; check: string } {
  switch (client) {
    case "claude":
      return {
        step: "Добавьте Genie как MCP-сервер в Claude Code:",
        snippet: `claude mcp add --transport http genie ${origin}/mcp \\\n  --header "Authorization: Bearer ${token}"`,
        check: "claude mcp list",
      };
    case "codex":
      return {
        step: "Допишите сервер в ~/.codex/config.toml, а токен положите в переменную GENIE_TOKEN:",
        snippet: `[mcp_servers.genie]\nurl = "${origin}/mcp"\nbearer_token_env_var = "GENIE_TOKEN"`,
        check: "codex mcp list",
      };
    case "cli":
      return {
        step: "Задайте переменные окружения для genie и своих скриптов:",
        snippet: `export GENIE_URL=${origin}\nexport GENIE_TOKEN=${token}`,
        check: "genie me show",
      };
  }
}

function day(iso: string): string {
  return new Date(iso).toLocaleDateString("ru-RU", { day: "numeric", month: "long", year: new Date(iso).getFullYear() === new Date().getFullYear() ? undefined : "numeric" });
}

export function TokensTab() {
  useTick(60_000);
  const tokens = useTokens(true);
  const issue = useIssueToken();
  const revoke = useRevokeToken();
  const act = useAct();
  const toast = useToast();
  const [label, setLabel] = useState("");
  const [fresh, setFresh] = useState<{ label: string; token: string }>();
  const [client, setClient] = useState<Client>("claude");
  const [revoking, setRevoking] = useState<UserToken>();
  const origin = window.location.origin;
  const how = howTo(client, origin, fresh?.token ?? "<токен>");
  const list = tokens.data ?? [];
  const copy = (text: string) =>
    void navigator.clipboard?.writeText(text).then(
      () => toast("Скопировано"),
      () => toast("Скопируйте вручную", "error"),
    );
  return (
    <div className="st-body split">
      <div className="st-col">
        <div className="st-head">
          <h2>Токены CLI и MCP</h2>
          <p>
            Токены дают доступ от вашего имени.
          </p>
        </div>

        <form
          className="pj-row"
          onSubmit={(e) => {
            e.preventDefault();
            const name = label.trim() || "без имени";
            void act(async () => {
              const r = await issue.mutateAsync(name);
              setFresh({ label: name, token: r.token });
              setLabel("");
            });
          }}
        >
          <input
            aria-label="Имя токена"
            placeholder="Где будет жить токен: ноутбук, CI, Claude Code…"
            value={label}
            maxLength={80}
            onChange={(e) => setLabel(e.target.value)}
            style={{ height: 36 }}
          />
          <button className="btn primary" style={{ height: 36 }} disabled={issue.isPending}>
            Выпустить токен
          </button>
        </form>

        {fresh && (
          <div className="st-ok" role="status">
            <span className="line">
              <Icon.check size={14} />
              <span>
                <b>Токен «{fresh.label}» выпущен.</b> Скопируйте его сейчас: больше мы его не покажем.
              </span>
            </span>
            <div className="st-secret">
              <code>{fresh.token}</code>
              <button type="button" className="btn" onClick={() => copy(fresh.token)}>
                Скопировать
              </button>
            </div>
          </div>
        )}

        <section className="st-card" aria-label="Выпущенные токены">
          <div className="st-table">
            <div className="tr th">
              <span>Имя</span>
              <span>Выдан</span>
              <span>Последнее использование</span>
              <span />
            </div>
            {list.map((t) => (
              <div key={t.id} className="tr">
                <span className="nm">
                  <span className="ic">
                    <Icon.key size={14} />
                  </span>
                  <b title={t.label}>{t.label || "без имени"}</b>
                </span>
                <span className="when">{day(t.created)}</span>
                <span className={`when${t.lastUsed ? "" : " idle"}`}>{t.lastUsed ? usedAgo(t.lastUsed) : "ещё не использовался"}</span>
                <span className="end">
                  <button type="button" className="btn revoke" aria-label={`Отозвать токен ${t.label}`} onClick={() => setRevoking(t)}>
                    Отозвать
                  </button>
                </span>
              </div>
            ))}
          </div>
          {!list.length && <div className="st-empty">{tokens.isPending ? "Загрузка…" : "Токенов нет."}</div>}

        </section>
      </div>

      <aside className="st-card" aria-label="Как подключить">
        <div className="st-card-body">
          <h3 style={{ margin: 0, fontSize: 13 }}>Как подключить</h3>
          <div className="st-switch" role="tablist" aria-label="Клиент">
            {CLIENTS.map((c) => (
              <button key={c.id} type="button" role="tab" aria-selected={client === c.id} className={client === c.id ? "on" : undefined} onClick={() => setClient(c.id)}>
                {c.title}
              </button>
            ))}
          </div>
          <ol className="st-steps">
            <li>
              <div>Выпустите токен и назовите его по устройству.</div>
            </li>
            <li>
              <div>
                <span>{how.step}</span>
                <pre>{how.snippet}</pre>
                <div>
                  <button type="button" className="btn sm" onClick={() => copy(how.snippet)}>
                    Скопировать
                  </button>
                </div>
              </div>
            </li>
            <li>
              <div>
                <span>Проверьте, что всё работает:</span>
                <pre>{how.check}</pre>
              </div>
            </li>
          </ol>
          <p className="muted" style={{ margin: 0, fontSize: 12, lineHeight: 1.5 }}>
            Токен даёт те же права, что у вас в проектах. Не кладите его в репозиторий.
          </p>
        </div>
      </aside>

      {revoking && (
        <ConfirmDialog
          title="Отозвать токен?"
          confirmLabel="Отозвать"
          danger
          busy={revoke.isPending}
          onClose={() => setRevoking(undefined)}
          onConfirm={() => void act(() => revoke.mutateAsync(revoking.id), `Токен «${revoking.label}» отозван`).then(() => setRevoking(undefined))}
        >
          Всё, что работает с токеном «{revoking.label}», перестанет достучаться до сервера. Вернуть его нельзя, только выпустить новый.
        </ConfirmDialog>
      )}
    </div>
  );
}

function usedAgo(iso: string): string {
  const ago = timeAgo(iso);
  if (ago === "сейчас") return "только что";
  return /^\d+ (мин|ч|д)$/.test(ago) ? `${ago} назад` : ago;
}
