// The role's prompt: a read-only view for people who do not edit the
// configuration, and the editor with a live preview that saves the file's body.

import { useState } from "react";
import { setBody, type RoleDef, type RoleDetail, splitFrontmatter, useSaveConfig } from "@/entities/agent-config";
import { Icon, Markdown, Modal, useToast } from "@/shared/ui";

/** The role's prompt to read (people who do not edit the configuration). */
export function PromptView({ role: r, onClose }: { role: RoleDef; onClose: () => void }) {
  return (
    <Modal label={`Промпт роли ${r.id}`} onClose={onClose} wide>
      <div className="mh">
        <Icon.file size={13} />
        <span className="ag-prompt-title">
          Промпт роли <span className="mono">{r.id}</span>
        </span>
        <button type="button" className="icon-btn" onClick={onClose} aria-label="Закрыть">
          <Icon.close />
        </button>
      </div>
      <div className="mb">
        {r.prompt ? <Markdown text={r.prompt} /> : <p className="muted">Промпта нет.</p>}
        {r.instructions && (
          <>
            <h4 className="ag-sub">Особенности роли</h4>
            <Markdown text={r.instructions} />
          </>
        )}
      </div>
    </Modal>
  );
}

/**
 * The role's prompt (the body of its file) with a live preview. An empty text
 * leaves the built-in prompt, or the parent role's, in force; the built-in text
 * is at hand to start from.
 */
export function PromptEditor({ detail, onClose }: { detail: RoleDetail; onClose: () => void }) {
  const r = detail.role;
  const file = detail.file.content;
  const builtin = detail.builtin === null ? undefined : (splitFrontmatter(detail.builtin)?.body ?? detail.builtin).trim();
  const [text, setText] = useState(() => (file === null ? "" : (splitFrontmatter(file)?.body ?? file).trim()));
  const [view, setView] = useState<"text" | "preview">("text");
  const [showBuiltin, setShowBuiltin] = useState(false);
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const save = useSaveConfig();
  const toast = useToast();
  const fallback = r.extends ? `промпт роли ${r.extends}` : builtin !== undefined ? "встроенный промпт" : undefined;
  const shown = text.trim() || (r.extends ? (r.prompt ?? "") : (builtin ?? ""));
  const submit = async () => {
    setBusy(true);
    try {
      await save("role", r.id, { content: setBody(file ?? "---\n---\n", text) }, detail.file.hash);
      toast("Промпт сохранён: агенты получат его со следующего старта сессии");
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal label={`Промпт роли ${r.id}`} onClose={onClose} wide>
      <div className="mh">
        <Icon.file size={13} />
        <span className="ag-prompt-title">
          Промпт роли <span className="mono">{r.id}</span>
        </span>
        <div className="seg ag-prompt-seg" role="tablist" aria-label="Вид">
          <button type="button" role="tab" aria-selected={view === "text"} className={view === "text" ? "on" : ""} onClick={() => setView("text")}>
            Текст
          </button>
          <button type="button" role="tab" aria-selected={view === "preview"} className={view === "preview" ? "on" : ""} onClick={() => setView("preview")}>
            Просмотр
          </button>
        </div>
        <button type="button" className="icon-btn" onClick={onClose} aria-label="Закрыть">
          <Icon.close />
        </button>
      </div>
      <div className="mb">
        <p className="muted ag-hint">
          Добавляется после общих правил genie.
          {fallback ? ` Пустой текст — действует ${fallback}.` : ""}
        </p>
        <div className={`ag-prompt-edit show-${view}`}>
          <textarea
            className="mono code-edit"
            value={text}
            onChange={(e) => setText(e.target.value)}
            spellCheck={false}
            aria-label="Текст промпта"
            placeholder={fallback ? `Пусто — ${fallback}` : "Кто ты в команде, как работаешь, что сдаёшь"}
          />
          <div className="ag-prompt-preview" aria-label="Предпросмотр">
            {!text.trim() && fallback && <div className="ag-hint muted">Сейчас действует {fallback}:</div>}
            {shown ? <Markdown text={shown} /> : <p className="muted">Промпта нет.</p>}
          </div>
        </div>
        {builtin !== undefined && (
          <details className="ag-prompt" open={showBuiltin} onToggle={(e) => setShowBuiltin(e.currentTarget.open)}>
            <summary>Встроенный текст роли</summary>
            <pre className="ag-file-text ag-builtin">{builtin}</pre>
            <button type="button" className="btn" onClick={() => setText(builtin)} disabled={text.trim() === builtin}>
              Взять встроенный
            </button>
          </details>
        )}
        {error && (
          <div className="auth-error" role="alert">
            {error}
          </div>
        )}
      </div>
      <div className="mf">
        {file === null ? "Сохранение создаст файл роли с этим промптом" : `agents/${r.id}.md`}
        <span className="grow" />
        <button type="button" className="btn ghost" onClick={onClose}>
          Отмена
        </button>
        <button type="button" className="btn primary" disabled={busy} onClick={() => void submit()}>
          Сохранить
        </button>
      </div>
    </Modal>
  );
}
