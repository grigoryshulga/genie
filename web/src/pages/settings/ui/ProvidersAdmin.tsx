// The «Провайдеры» tab of the server settings, for its administrators: the
// provider keys the server shares with everyone — the LiteLLM proxy key an
// agent owes, and the keys of the providers pi supports. A person's own key of
// the same provider comes first; where they keep none, their agents run on the
// shared one.

import { useState } from "react";
import { useSharedLlmKeys, type LlmProvider } from "@/entities/platform";
import { request } from "@/shared/api";
import { timeAgo } from "@/shared/lib";
import { useRefreshingAct as useAct } from "@/shared/ui";
import "@/shared/ui/settings.css";

export function ProvidersAdminSection() {
  const { data } = useSharedLlmKeys();
  const act = useAct();
  const [values, setValues] = useState<Record<string, string>>({});
  const [adding, setAdding] = useState("");
  const providers = data?.providers ?? [];
  const keys = data?.keys ?? {};
  const strict = providers.find((p) => p.strict);
  const rest = providers.filter((p) => !p.strict);
  const shown = rest.filter((p) => keys[p.id] || p.id === adding);
  const unset = rest.filter((p) => !keys[p.id]);

  const save = (p: LlmProvider, key: string) =>
    act(async () => {
      await request("PUT", `/api/llm-keys/${p.id}`, { key });
      setValues((v) => ({ ...v, [p.id]: "" }));
      setAdding("");
    }, `Ключ ${p.title} сохранён`);

  const card = (p: LlmProvider) => {
    const key = keys[p.id];
    const value = values[p.id] ?? "";
    return (
      <form
        className="st-card"
        key={p.id}
        onSubmit={(e) => {
          e.preventDefault();
          void save(p, value);
        }}
      >
        <div className="st-row">
          <div className="lbl">
            <b>{p.title}</b>
            <span>
              {p.strict
                ? "Прокси этого сервера. Агенты на моделях litellm/… людей без своего ключа запускаются на нём; ждущие агенты стартуют сразу после сохранения."
                : `Провайдер pi: агенты на моделях этого провайдера (${p.id}/…) людей без своего ключа получают его в переменной ${p.keyVar} — поверх серверного окружения.`}
            </span>
          </div>
        </div>
        {key && (
          <div className="st-row">
            <div className="lbl">
              <b>Сейчас</b>
              <span>Ключ хранится на сервере в зашифрованном виде и больше не показывается.</span>
            </div>
            <div className="ctl" style={{ flexDirection: "row", alignItems: "center", flexWrap: "wrap" }}>
              {key.unreadable ? (
                <span className="pj-bad">Ключ сохранён, но сервер не может его прочитать (сменился ключ шифрования). Укажите его заново.</span>
              ) : (
                <span>
                  Ключ задан{key.hint ? ` (${key.hint})` : ""}, обновлён {timeAgo(key.updated)}.
                </span>
              )}
              <button
                type="button"
                className="btn ghost"
                onClick={() => void act(() => request("DELETE", `/api/llm-keys/${p.id}`), `Ключ ${p.title} удалён`)}
              >
                Удалить
              </button>
            </div>
          </div>
        )}
        <div className="st-row">
          <label className="lbl" htmlFor={`sv-key-${p.id}`}>
            <b>{key ? "Новый ключ" : "Ключ"}</b>
            <span>Агенты людей без своего ключа ходят к провайдеру с ним; личный ключ человека перекрывает его.</span>
          </label>
          <div className="ctl">
            <input
              id={`sv-key-${p.id}`}
              type="password"
              className="st-input"
              autoComplete="off"
              placeholder="sk-…"
              value={value}
              onChange={(e) => setValues((v) => ({ ...v, [p.id]: e.target.value }))}
              required
            />
          </div>
        </div>
        <div className="st-foot">
          {!key && p.id === adding && (
            <button type="button" className="btn ghost" onClick={() => setAdding("")}>
              Отмена
            </button>
          )}
          <span className="grow" />
          <button className="btn primary" disabled={!value.trim()}>
            {key ? `Заменить ключ ${p.title}` : `Сохранить ключ ${p.title}`}
          </button>
        </div>
      </form>
    );
  };

  return (
    <div className="st-body">
      <div className="st-head">
        <h2>Провайдеры и ключи сервера</h2>
        <p>
          Общие ключи для всех пользователей. Личный ключ имеет приоритет. Значения хранятся зашифрованными.
        </p>
      </div>
      {strict && card(strict)}
      {shown.map(card)}
      {unset.length > 0 && (
        <div className="st-card">
          <div className="st-row">
            <label className="lbl" htmlFor="sv-add-provider">
              <b>Другой провайдер</b>
            </label>
            <div className="ctl">
              <select id="sv-add-provider" className="st-input" value={adding} onChange={(e) => setAdding(e.target.value)}>
                <option value="">— выбрать провайдера —</option>
                {unset.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.title}
                  </option>
                ))}
              </select>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
