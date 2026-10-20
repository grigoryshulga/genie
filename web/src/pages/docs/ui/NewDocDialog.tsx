// NewDocDialog — "Новая страница / Быстрая заметка" (mockup 7). Creates the file
// on the server (`mode: "create"`) and opens it in the editor. The note variant
// mirrors `genie docs note`: `inbox/<date>-<slug>.md`, type note, status draft.

import { type KeyboardEvent, useMemo, useState } from "react";
import { articlePath, docSections, sectionTrail, type DocSection, diagnosticText, DOC_TYPE_NAME, DOC_TYPES, DocSaveError, useSaveDoc, serializeDoc, slugify, todayIso, type DocType } from "@/entities/doc";
import { Icon } from "@/shared/ui";

export function NewDocDialog({
  navigation,
  initialSection,
  initialKind = "page",
  initialTitle = "",
  onClose,
  onCreated,
}: {
  navigation: DocSection;
  initialSection?: string;
  initialKind?: "page" | "note" | "section";
  initialTitle?: string;
  onClose: () => void;
  onCreated: (path: string) => void;
}) {
  const [kind, setKind] = useState<"page" | "note" | "section">(initialKind);
  const [title, setTitle] = useState(initialTitle);
  const [section, setSection] = useState(initialSection || navigation.path || navigation.sections[0]?.path || "");
  const save = useSaveDoc();
  const sections = docSections(navigation).filter((s) => s.path);
  const [type, setType] = useState<DocType | "">(initialKind === "note" ? "note" : "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const docPath = useMemo(() => articlePath(section, slugify(title || "page"), kind, todayIso()), [title, section, kind]);

  const switchKind = (next: "page" | "note" | "section") => {
    setKind(next);
    setType(next === "note" ? "note" : "");
  };

  const create = async () => {
    if (!title.trim() || !sections.some((s) => s.path === section) || busy) return;
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
      await save.mutateAsync({ path: docPath, content, mode: "create" });
      onCreated(docPath);
    } catch (cause) {
      if (cause instanceof DocSaveError) {
        if (cause.status === 409) setError("Статья с таким названием уже существует в этом разделе — измените название или раздел.");
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
      <div className="modal doc-new" role="dialog" aria-modal="true" aria-label="Создание статьи или раздела" onKeyDown={onKeyDown}>
        <div className="mh">
          <div className="seg doc-tabs" role="tablist">
            <button type="button" role="tab" aria-selected={kind === "page"} className={kind === "page" ? "on" : ""} onClick={() => switchKind("page")}>
              Статья
            </button>
            <button type="button" role="tab" aria-selected={kind === "section"} className={kind === "section" ? "on" : ""} onClick={() => switchKind("section")}>Раздел</button>
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
              <span>{kind === "section" ? "Родительский раздел" : "Раздел"}</span>
              <select value={section} onChange={(e) => setSection(e.target.value)} aria-label="Раздел">
                {!sections.length && <option value="">Нет доступных разделов</option>}
                {sections.map((s) => <option key={s.path} value={s.path}>{sectionTrail(navigation, s.path).map((part, i) => i === 0 ? "Пространство" : part.title).join(" › ")}</option>)}
              </select>
            </label>
            <label className="doc-field">
              <span>Тип</span>
              <select value={kind === "note" ? "note" : type} disabled={kind === "note"} onChange={(e) => setType(e.target.value as DocType | "")}>
                {kind !== "note" && <option value="">не указан</option>}
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
              ? "Черновик заметки в выбранном разделе."
              : kind === "section" ? "Раздел объединяет статьи и вложенные темы." : "Статья будет создана в выбранном разделе."}
          </p>

          {kind === "section" && <p className="doc-new-hint">Название обзорной статьи станет названием раздела в навигации.</p>}
          {error && <p className="doc-new-error">{error}</p>}
        </div>

        <div className="mf">
          <span>Откроется в редакторе</span>
          <span className="grow" />
          <button type="button" className="btn" onClick={onClose}>
            Отмена
          </button>
          <button type="button" className="btn primary" disabled={!title.trim() || !sections.some((s) => s.path === section) || busy} onClick={() => void create()}>
            <Icon.check size={13} />
            {busy ? "Создаю…" : "Создать"}
          </button>
        </div>
      </div>
    </div>
  );
}
