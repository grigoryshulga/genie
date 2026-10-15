// NewDocDialog — "Новая страница / Быстрая заметка" (mockup 7). Creates the file
// on the server (`mode: "create"`) and opens it in the editor. The note variant
// mirrors `genie docs note`: `inbox/<date>-<slug>.md`, type note, status draft.

import { type KeyboardEvent, useMemo, useState } from "react";
import { diagnosticText, DOC_TYPE_NAME, DOC_TYPES, DocSaveError, saveDoc, serializeDoc, slugify, todayIso, type DocType } from "@/entities/doc";
import { Icon } from "@/shared/ui";

export function NewDocDialog({
  initialKind = "page",
  initialTitle = "",
  onClose,
  onCreated,
}: {
  initialKind?: "page" | "note";
  initialTitle?: string;
  onClose: () => void;
  onCreated: (path: string) => void;
}) {
  const [kind, setKind] = useState<"page" | "note">(initialKind);
  const [title, setTitle] = useState(initialTitle);
  const [folder, setFolder] = useState(initialKind === "note" ? "inbox" : "");
  const [type, setType] = useState<DocType | "">(initialKind === "note" ? "note" : "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const docPath = useMemo(() => {
    const slug = slugify(title || "page");
    const file = kind === "note" ? `${todayIso()}-${slug}.md` : `${slug}.md`;
    const clean = folder.trim().replace(/^\/+|\/+$/g, "");
    return clean ? `${clean}/${file}` : file;
  }, [title, folder, kind]);

  const switchKind = (next: "page" | "note") => {
    setKind(next);
    setFolder(next === "note" ? "inbox" : "");
    setType(next === "note" ? "note" : "");
  };

  const create = async () => {
    if (!title.trim() || busy) return;
    setBusy(true);
    setError(null);
    const content = serializeDoc(
      {
        title: title.trim(),
        type: kind === "note" ? "note" : type,
        status: "draft",
        verified: "",
        tags: [],
        paths: [],
        related: [],
        summary: null,
        aliases: [],
      },
      `# ${title.trim()}\n\n`,
    );
    try {
      await saveDoc({ path: docPath, content, mode: "create" });
      onCreated(docPath);
    } catch (cause) {
      if (cause instanceof DocSaveError) {
        if (cause.status === 409) setError(`Страница docs/${docPath} уже существует — измените название или папку.`);
        else if (cause.status === 422) setError(`Frontmatter не принят сервером: ${cause.diagnostics.map(diagnosticText).join("; ")}`);
        else setError(cause.message);
      } else {
        setError(cause instanceof Error ? cause.message : String(cause));
      }
      setBusy(false);
    }
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) void create();
  };

  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal doc-new" role="dialog" aria-modal="true" aria-label="Новая страница или заметка" onKeyDown={onKeyDown}>
        <div className="mh">
          <div className="seg doc-tabs" role="tablist">
            <button type="button" role="tab" aria-selected={kind === "page"} className={kind === "page" ? "on" : ""} onClick={() => switchKind("page")}>
              Страница
            </button>
            <button type="button" role="tab" aria-selected={kind === "note"} className={kind === "note" ? "on" : ""} onClick={() => switchKind("note")}>
              Быстрая заметка
            </button>
          </div>
          <span className="grow" />
          <button type="button" className="icon-btn" onClick={onClose} aria-label="Закрыть">
            <Icon.close size={14} />
          </button>
        </div>

        <div className="mb">
          <label className="doc-field">
            <span>Название</span>
            <input autoFocus value={title} placeholder="Например: Как устроен деплой" onChange={(e) => setTitle(e.target.value)} />
          </label>
          <div className="doc-new-row">
            <label className="doc-field">
              <span>Папка</span>
              <div className="doc-folder-in">
                <Icon.file size={13} />
                <input className="mono" value={folder} placeholder="docs/" onChange={(e) => setFolder(e.target.value)} />
              </div>
            </label>
            <label className="doc-field">
              <span>Тип</span>
              <select value={kind === "note" ? "note" : type} disabled={kind === "note"} onChange={(e) => setType(e.target.value as DocType | "")}>
                {kind === "page" && <option value="">не указан</option>}
                {DOC_TYPES.map((t) => (
                  <option key={t} value={t}>
                    {t} — {DOC_TYPE_NAME[t]}
                  </option>
                ))}
              </select>
            </label>
          </div>

          <p className="doc-new-hint">
            {kind === "note"
              ? "Черновик в inbox/."
              : "Проверка изменений кода доступна после отметки «Проверено»."}
          </p>

          <p className="doc-new-file">
            Файл: <span className="mono">docs/{docPath}</span>
          </p>
          {error && <p className="doc-new-error">{error}</p>}
        </div>

        <div className="mf">
          <span>Откроется в редакторе</span>
          <span className="grow" />
          <button type="button" className="btn" onClick={onClose}>
            Отмена
          </button>
          <button type="button" className="btn primary" disabled={!title.trim() || busy} onClick={() => void create()}>
            <Icon.check size={13} />
            {busy ? "Создаю…" : "Создать"}
          </button>
        </div>
      </div>
    </div>
  );
}
