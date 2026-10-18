// Article metadata and save/conflict handling; content editing lives in DocContentEditor.
// `read().content` is the body without frontmatter, so the form is serialized
// back into `---…---` + body on save.

import { type KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router";
import type { DocFields, DocReadResult, DocStatus, DocType } from "@/entities/doc";
import {
  diagnosticText,
  DOC_STATUS_NAME,
  DOC_TYPE_NAME,
  DOC_STATUSES,
  DOC_TYPES,
  DocRequestError,
  DocSaveError,
  articleTitle,
  fieldsFromPage,
  isIsoDate,
  readDocPage,
  serializeDoc,
  useDocPage,
  useDocsVersion,
  useSaveDoc,
} from "@/entities/doc";
import { DocContentEditor } from "./DocContentEditor.tsx";
import { DiagIcon, StaleIcon } from "@/entities/doc";
import { plural } from "@/shared/lib";
import { Icon } from "@/shared/ui";

function countChanges(before: string, after: string): number {
  const a = before.replace(/\r/g, "").split("\n");
  const b = after.replace(/\r/g, "").split("\n");
  let changes = 0;
  for (let i = 0; i < Math.max(a.length, b.length); i++) if (a[i] !== b[i]) changes++;
  return changes;
}

/** Minimal chip list editor for `tags`, `paths` and `related`. */
function Chips({ value, onChange, placeholder, mono }: { value: string[]; onChange: (next: string[]) => void; placeholder: string; mono?: boolean }) {
  const [draft, setDraft] = useState("");
  const add = (raw: string) => {
    const item = raw.trim().replace(/,$/, "");
    if (!item) return;
    if (!value.includes(item)) onChange([...value, item]);
    setDraft("");
  };
  return (
    <div className={`doc-chips${mono ? " mono" : ""}`}>
      {value.map((item) => (
        <span className="doc-chip" key={item}>
          {item}
          <button type="button" onClick={() => onChange(value.filter((v) => v !== item))} aria-label={`Убрать ${item}`}>
            <Icon.close size={9} />
          </button>
        </span>
      ))}
      <input
        value={draft}
        placeholder={value.length ? "" : placeholder}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => add(draft)}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === ",") {
            e.preventDefault();
            add(draft);
          } else if (e.key === "Backspace" && !draft && value.length) {
            onChange(value.slice(0, -1));
          }
        }}
      />
    </div>
  );
}

export function DocEditor({ path, pages, onClose, onSaved }: { path: string; pages: { path: string; title: string }[]; onClose: () => void; onSaved: (path: string) => void }) {
  const pageQ = useDocPage(path);
  const save = useSaveDoc();
  const [fields, setFields] = useState<DocFields | null>(null);
  const [body, setBody] = useState("");
  const [initial, setInitial] = useState<{ fields: DocFields; body: string } | null>(null);
  const [serverDiags, setServerDiags] = useState<string[]>([]);
  const [saveError, setSaveError] = useState<string | null>(null);
  /**
   * `gone` = 404, the file vanished; `exists` = 409, somebody created it;
   * `lost-update` = the file changed on disk since it was loaded.
   */
  const [conflict, setConflict] = useState<{ kind: "gone" | "exists" | "lost-update"; disk?: DocReadResult } | null>(null);
  const loadedFor = useRef<string | undefined>(undefined);

  // Docs change signal: a moved version means the file on disk is ahead of the editor.
  const versionQ = useDocsVersion();
  const version = versionQ.data?.version;
  const versionRef = useRef<string | undefined>(undefined);
  const [baseline, setBaseline] = useState<string | undefined>(undefined);
  useEffect(() => {
    versionRef.current = version;
  }, [version]);

  const page: DocReadResult | undefined = pageQ.data;
  useEffect(() => {
    if (!page || loadedFor.current === page.path) return;
    loadedFor.current = page.path;
    const next = fieldsFromPage(page);
    setFields(next);
    setInitial({ fields: next, body: page.content });
    setBody(page.content);
    setServerDiags([]);
    setSaveError(null);
    setBaseline(versionRef.current);
  }, [page]);

  const title = pages.find((p) => p.path === path)?.title ?? page?.title ?? path;
  const errors = useMemo(() => {
    if (!fields) return [];
    const out: { field: string; message: string }[] = [];
    if (!fields.title.trim()) out.push({ field: "title", message: "Нужно название страницы" });
    if (fields.verified.trim() && !isIsoDate(fields.verified.trim())) out.push({ field: "verified", message: "Нужна дата ГГГГ-ММ-ДД" });
    return out;
  }, [fields]);
  const errorFor = (field: string) => errors.find((e) => e.field === field)?.message;

  const dirty = !!fields && !!initial && (body !== initial.body || JSON.stringify(fields) !== JSON.stringify(initial.fields));
  const changedLines = initial && body !== initial.body ? countChanges(initial.body, body) : 0;
  /** Only a real move of the docs signature after this page was loaded counts. */
  const externallyChanged = !!baseline && !!version && version !== baseline;

  // The version query may resolve after the page did; adopt it as the baseline once.
  useEffect(() => {
    if (fields && baseline === undefined && version !== undefined) setBaseline(version);
  }, [fields, baseline, version]);

  /** Serialized form of a read state: the comparable fingerprint of the file. */
  const fingerprint = (state: { fields: DocFields; body: string }) => serializeDoc(state.fields, state.body);

  /**
   * Lost-update guard: the API has no optimistic-concurrency token, so the disk
   * copy is re-read and compared with what the editor loaded. Returns true when a
   * conflict is on screen (and the caller must not save).
   */
  const checkDisk = async (): Promise<boolean> => {
    if (!initial) return false;
    try {
      const fresh = await readDocPage(path);
      if (fingerprint({ fields: fieldsFromPage(fresh), body: fresh.content }) !== fingerprint(initial)) {
        setConflict({ kind: "lost-update", disk: fresh });
        return true;
      }
      setBaseline(versionRef.current);
      return false;
    } catch (error) {
      if (error instanceof DocRequestError && error.status === 404) {
        setConflict({ kind: "gone" });
        return true;
      }
      return false;
    }
  };

  // The page disappeared (or became unreadable) after it was loaded: keep the
  // editor content and surface it as a save conflict instead of losing the edits.
  useEffect(() => {
    if (!fields || !pageQ.isError) return;
    const status = pageQ.error instanceof DocRequestError ? pageQ.error.status : 0;
    if (status === 404 || status === 0) setConflict((current) => current ?? { kind: "gone" });
  }, [fields, pageQ.isError, pageQ.error]);

  // The change signal moved: find out whether *this* page changed and warn
  // before the author saves over it. Other files moving only reset the baseline.
  useEffect(() => {
    if (!externallyChanged || conflict || save.isPending) return;
    void checkDisk();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [externallyChanged, conflict, save.isPending]);

  const submit = async (mode: "update" | "create" = "update", force = false) => {
    if (!fields || errors.length) return;
    if (!body.trim()) {
      setSaveError("Содержимое страницы пустое — сервер не примет такое сохранение");
      return;
    }
    setSaveError(null);
    setServerDiags([]);
    setConflict(null);
    // Never overwrite silently: re-read the file first (mode "update" only).
    if (mode === "update" && !force && (await checkDisk())) return;
    try {
      await save.mutateAsync({ path, content: serializeDoc(fields, body), mode });
      onSaved(path);
    } catch (error) {
      if (error instanceof DocSaveError) {
        // 404 = the file was deleted/moved, 409 = somebody created it meanwhile.
        if (error.status === 404 || error.status === 409) {
          setConflict(error.status === 409 ? { kind: "exists" } : { kind: "gone" });
          return;
        }
        setServerDiags(error.diagnostics);
        setSaveError(error.diagnostics.length ? "Frontmatter не принят сервером — исправьте поля ниже" : error.message);
      } else {
        setSaveError(error instanceof Error ? error.message : String(error));
      }
    }
  };

  /** Re-read the page from disk, dropping the local form/body state. */
  const reload = async () => {
    let freshPage: DocReadResult | undefined = conflict?.kind === "lost-update" ? conflict.disk : undefined;
    if (!freshPage) {
      const fresh = await pageQ.refetch();
      freshPage = fresh.data;
    }
    if (!freshPage) {
      setSaveError("Страница недоступна на диске");
      return;
    }
    const next = fieldsFromPage(freshPage);
    setFields(next);
    setInitial({ fields: next, body: freshPage.content });
    setBody(freshPage.content);
    setServerDiags([]);
    setSaveError(null);
    setConflict(null);
    setBaseline(versionRef.current);
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
      event.preventDefault();
      void submit();
    }
  };

  if (!fields) {
    if (pageQ.isError) return <div className="docs-content"><div className="doc-editor-wait muted">Не удалось открыть страницу: {pageQ.error.message}</div></div>;
    return <div className="docs-content"><div className="doc-editor-wait muted">Загрузка страницы…</div></div>;
  }

  const update = <K extends keyof DocFields>(key: K, value: DocFields[K]) => setFields({ ...fields, [key]: value });

  return (
    <div className="docs-editor" onKeyDown={onKeyDown}>
      <header className="docs-topbar">
        <button type="button" className="icon-btn" onClick={onClose} aria-label="Вернуться к странице">
          <Icon.back size={15} />
        </button>
        <nav className="doc-crumbs" aria-label="Навигация по документации"><Link to="/docs">Документация</Link><span className="sep">›</span>{articleTitle({ path, title })}</nav>
        {dirty ? (
          <span className="doc-chip-state" title="Изменения не сохранены">
            <i className="dot" />
            Не сохранено{changedLines > 0 ? ` · ${changedLines} ${plural(changedLines, "строка", "строки", "строк")} изменения` : ""}
          </span>
        ) : (
          <span className="doc-chip-state clean">
            <Icon.check size={11} />
            Сохранено
          </span>
        )}
        <span className="grow" />
        {externallyChanged && (
          <span className="doc-chip-state warn" title="Файл на диске изменился после открытия редактора">
            <StaleIcon size={11} />
            Изменено на диске
          </span>
        )}
        {errors.length > 0 && (
          <span className="doc-chip-state error">
            {errors.length} {plural(errors.length, "поле", "поля", "полей")} с ошибкой
          </span>
        )}
        <Link className="btn" to={`/docs?page=${encodeURIComponent(path)}`}>
          Отмена
        </Link>
        <button type="button" className="btn primary" onClick={() => void submit()} disabled={save.isPending || errors.length > 0}>
          <Icon.check size={13} />
          {save.isPending ? "Сохраняю…" : "Сохранить"}
        </button>
      </header>

      {conflict?.kind === "gone" && (
        <div className="doc-banner diag editor">
          <DiagIcon size={15} />
          <div className="doc-banner-body">
            <b>Конфликт сохранения: страница исчезла.</b> Файла <span className="mono">{path}</span> больше нет на диске — его удалили или переместили после открытия редактора.
          </div>
          <button type="button" className="btn" onClick={() => void reload()}>
            Перечитать
          </button>
          <button type="button" className="btn primary" onClick={() => void submit("create", true)}>
            Создать заново
          </button>
        </div>
      )}

      {conflict?.kind === "exists" && (
        <div className="doc-banner diag editor">
          <DiagIcon size={15} />
          <div className="doc-banner-body">
            <b>Конфликт сохранения: страница уже существует.</b> <span className="mono">{path}</span> создали параллельно — откройте текущую версию, чтобы не перезаписать чужие правки.
          </div>
          <Link className="btn" to={`/docs?page=${encodeURIComponent(path)}`}>
            Открыть текущую версию
          </Link>
        </div>
      )}

      {conflict?.kind === "lost-update" && (
        <div className="doc-banner stale editor">
          <StaleIcon size={15} />
          <div className="doc-banner-body">
            <b>Страница изменилась на диске</b> после того, как редактор её открыл (сигнал обновления документации). Чтобы не перезаписать чужие правки, выберите, что делать.
          </div>
          <button type="button" className="btn" onClick={() => void reload()} title="Ваши правки в форме и тексте будут потеряны">
            Обновить из файла
          </button>
          <button type="button" className="btn primary" onClick={() => void submit("update", true)} title="Сохранить вашу версию поверх файла на диске">
            Перезаписать
          </button>
        </div>
      )}

      {saveError && (
        <div className="doc-banner diag editor">
          <span>
            <b>{saveError}</b>
            {serverDiags.length > 0 && (
              <ul className="doc-banner-list">
                {serverDiags.map((raw) => (
                  <li key={raw} title={raw} className="mono">
                    {diagnosticText(raw)}
                  </li>
                ))}
              </ul>
            )}
          </span>
        </div>
      )}

      <div className="doc-form">
        <label className="doc-field span6">
          <span>Название статьи</span>
          <input className={errorFor("title") ? "bad" : ""} value={fields.title} onChange={(e) => update("title", e.target.value)} />
          {errorFor("title") && <em className="err">{errorFor("title")}</em>}
        </label>
        <label className="doc-field span2">
          <span>Тип</span>
          <select value={fields.type} onChange={(e) => update("type", e.target.value as DocType | "")}>
            <option value="">не указан</option>
            {DOC_TYPES.map((type) => (
              <option key={type} value={type}>
                {DOC_TYPE_NAME[type]}
              </option>
            ))}
          </select>
        </label>
        <label className="doc-field span2">
          <span>Статус</span>
          <select value={fields.status} onChange={(e) => update("status", e.target.value as DocStatus | "")}>
            <option value="">не указан</option>
            {DOC_STATUSES.map((status) => (
              <option key={status} value={status}>
                {DOC_STATUS_NAME[status]}
              </option>
            ))}
          </select>
        </label>
        <label className="doc-field span2">
          <span>Проверена</span>
          <input
            className={errorFor("verified") ? "bad" : ""}
            placeholder="ГГГГ-ММ-ДД"
            value={fields.verified}
            onChange={(e) => update("verified", e.target.value)}
          />
          {errorFor("verified") && <em className="err">{errorFor("verified")}</em>}
        </label>

        <div className="doc-field span6">
          <span>Теги</span>
          <Chips value={fields.tags} onChange={(next) => update("tags", next)} placeholder="+ тег" />
        </div>
        <div className="doc-field span4">
          <span>Код, который описывает статья</span>
          <Chips mono value={fields.paths} onChange={(next) => update("paths", next)} placeholder="+ glob" />
        </div>
        <div className="doc-field span2">
          <span>Связанные задачи</span>
          <Chips mono value={fields.related} onChange={(next) => update("related", next)} placeholder="+ id" />
        </div>
      </div>

      <DocContentEditor value={body} onChange={setBody} title={fields.title || title} />
    </div>
  );
}
