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
  fieldsFromPage,
  isIsoDate,
  readDocPage,
  serializeDoc,
  useDocPage,
  useDocsVersion,
  useSaveDoc,
} from "@/entities/doc";
import { DocContentEditor } from "./DocContentEditor.tsx";
import "./editor.css";
import { DiagIcon, StaleIcon } from "@/entities/doc";
import { plural } from "@/shared/lib";
import { Button, Dialog, DisclosureCard, Icon, LoadingState } from "@/shared/ui";

function countChanges(before: string, after: string): number {
  const a = before.replace(/\r/g, "").split("\n");
  const b = after.replace(/\r/g, "").split("\n");
  let changes = 0;
  for (let i = 0; i < Math.max(a.length, b.length); i++) if (a[i] !== b[i]) changes++;
  return changes;
}

/** Minimal chip list editor for `tags`, `paths` and `related`. */
function Chips({ value, onChange, placeholder, mono, label }: { value: string[]; onChange: (next: string[]) => void; placeholder: string; mono?: boolean; label: string }) {
  const [draft, setDraft] = useState("");
  const add = (raw: string) => {
    const item = raw.trim().replace(/,$/, "");
    if (!item) return;
    if (!value.includes(item)) onChange([...value, item]);
    setDraft("");
  };
  return (
    <div className={`doc-chips${mono ? " g-mono" : ""}`}>
      {value.map((item) => (
        <span className="doc-chip" key={item}>
          {item}
          <button type="button" onClick={() => onChange(value.filter((v) => v !== item))} aria-label={`Убрать ${item}`}>
            <Icon.close size={9} />
          </button>
        </span>
      ))}
      <input
        aria-label={label}
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

export function DocEditor({ path, pages, onSaved }: { path: string; pages: { path: string; title: string }[]; onSaved: (path: string, changeset?: number) => void }) {
  const [narrow, setNarrow] = useState(() => window.matchMedia("(max-width: 900px)").matches);
  const [propertiesOpen, setPropertiesOpen] = useState(false);
  useEffect(() => {
    const query = window.matchMedia("(max-width: 900px)");
    const change = () => setNarrow(query.matches);
    query.addEventListener("change", change);
    return () => query.removeEventListener("change", change);
  }, []);
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
      const saved = await save.mutateAsync({ path, content: serializeDoc(fields, body), mode });
      onSaved(path, saved.changeset?.id);
    } catch (error) {
      if (error instanceof DocSaveError) {
        // 404 = the page was deleted/moved; 409 = the changeset is in the wrong state
        // (or, for a create, somebody created the page meanwhile); 403 = locked or not the owner.
        if (error.status === 404) {
          setConflict({ kind: "gone" });
          return;
        }
        if (error.status === 409) {
          if (mode === "create") {
            setConflict({ kind: "exists" });
            return;
          }
          setSaveError("Черновик изменился: его уже опубликовали или отменили. Обновите страницу и повторите правку.");
          return;
        }
        if (error.status === 403) {
          setSaveError("Сохранить нельзя: страница заблокирована или вы не владелец раздела.");
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
    if (pageQ.isError) return <div className="docs-content"><div className="doc-editor-wait g-muted">Не удалось открыть страницу: {pageQ.error.message}</div></div>;
    return <div className="docs-content"><div className="doc-editor-wait"><LoadingState label="Загрузка страницы" lines={4} width={520} /></div></div>;
  }

  const update = <K extends keyof DocFields>(key: K, value: DocFields[K]) => setFields({ ...fields, [key]: value });

  const properties = (
    <div className="doc-editor-properties">
      <DisclosureCard title="Статья" storageKey="genie.docs.editor.article">
        <div className="doc-editor-fields">
        <label className="doc-field">
          <span>Название статьи</span>
          <input className={`g-input${errorFor("title") ? " bad" : ""}`} value={fields.title} onChange={(e) => update("title", e.target.value)} />
          {errorFor("title") && <em className="err">{errorFor("title")}</em>}
        </label>
        <label className="doc-field">
          <span>Тип</span>
          <select aria-label="Тип" className="g-select" value={fields.type} onChange={(e) => update("type", e.target.value as DocType | "")}>
            <option value="">не указан</option>
            {DOC_TYPES.map((type) => (
              <option key={type} value={type}>
                {DOC_TYPE_NAME[type]}
              </option>
            ))}
          </select>
        </label>
        <label className="doc-field">
          <span>Статус</span>
          <select aria-label="Статус" className="g-select" value={fields.status} onChange={(e) => update("status", e.target.value as DocStatus | "")}>
            <option value="">не указан</option>
            {DOC_STATUSES.map((status) => (
              <option key={status} value={status}>
                {DOC_STATUS_NAME[status]}
              </option>
            ))}
          </select>
        </label>
        <label className="doc-field">
          <span>Проверена</span>
          <input
            className={`g-input${errorFor("verified") ? " bad" : ""}`}
            placeholder="ГГГГ-ММ-ДД"
            value={fields.verified}
            onChange={(e) => update("verified", e.target.value)}
          />
          {errorFor("verified") && <em className="err">{errorFor("verified")}</em>}
        </label>

        </div>
      </DisclosureCard>
      <DisclosureCard title="Теги" storageKey="genie.docs.editor.tags">
        <Chips label="Теги" value={fields.tags} onChange={(next) => update("tags", next)} placeholder="+ тег" />
      </DisclosureCard>
      <DisclosureCard title="Связи" storageKey="genie.docs.editor.links">
        <div className="doc-editor-fields">
        <div className="doc-field">
          <span>Код, который описывает статья</span>
          <Chips label="Код, который описывает статья" mono value={fields.paths} onChange={(next) => update("paths", next)} placeholder="+ glob" />
        </div>
        <div className="doc-field">
          <span>Связанные задачи</span>
          <Chips label="Связанные задачи" mono value={fields.related} onChange={(next) => update("related", next)} placeholder="+ id" />
        </div>
        </div>
      </DisclosureCard>
    </div>
  );

  return (
    <div className="docs-editor doc-editor-fullscreen" onKeyDown={onKeyDown}>
      {conflict?.kind === "gone" && (
        <div className="doc-banner diag editor g-banner danger">
          <DiagIcon size={15} />
          <div className="doc-banner-body">
            <b>Конфликт сохранения: страница исчезла.</b> Файла <span className="g-mono">{path}</span> больше нет на диске — его удалили или переместили после открытия редактора.
          </div>
          <Button sm onClick={() => void reload()}>
            Перечитать
          </Button>
          <Button variant="primary" sm onClick={() => void submit("create", true)}>
            Создать заново
          </Button>
        </div>
      )}

      {conflict?.kind === "exists" && (
        <div className="doc-banner diag editor g-banner danger">
          <DiagIcon size={15} />
          <div className="doc-banner-body">
            <b>Конфликт сохранения: страница уже существует.</b> <span className="g-mono">{path}</span> создали параллельно — откройте текущую версию, чтобы не перезаписать чужие правки.
          </div>
          <Link className="g-btn secondary" to={`/docs?page=${encodeURIComponent(path)}`}>
            Открыть текущую версию
          </Link>
        </div>
      )}

      {conflict?.kind === "lost-update" && (
        <div className="doc-banner stale editor g-banner accent">
          <StaleIcon size={15} />
          <div className="doc-banner-body">
            <b>Страница изменилась на диске</b> после того, как редактор её открыл (сигнал обновления документации). Чтобы не перезаписать чужие правки, выберите, что делать.
          </div>
          <Button sm onClick={() => void reload()} title="Ваши правки в форме и тексте будут потеряны">
            Обновить из файла
          </Button>
          <Button variant="primary" sm onClick={() => void submit("update", true)} title="Сохранить вашу версию поверх файла на диске">
            Перезаписать
          </Button>
        </div>
      )}

      {saveError && (
        <div className="doc-banner diag editor g-banner danger">
          <span>
            <b>{saveError}</b>
            {serverDiags.length > 0 && (
              <ul className="doc-banner-list">
                {serverDiags.map((raw) => (
                  <li key={raw} title={raw} className="g-mono">
                    {diagnosticText(raw)}
                  </li>
                ))}
              </ul>
            )}
          </span>
        </div>
      )}

      <div className="doc-editor-workspace">
        <div className="doc-editor-canvas">
          <DocContentEditor value={body} onChange={setBody} title={fields.title || title} />
        </div>
        {!narrow && <aside className="doc-editor-sidebar" aria-label="Свойства статьи">{properties}</aside>}
      </div>
      {narrow && propertiesOpen && <Dialog title="Свойства статьи" sheet onClose={() => setPropertiesOpen(false)}>{properties}</Dialog>}
      {/* Save state and actions sit at the bottom: the status of the text, then the way out and the way in. */}
      <footer className="doc-editor-footer">
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
        {narrow && <Button sm aria-expanded={propertiesOpen} onClick={() => setPropertiesOpen(true)}>Свойства статьи</Button>}
        <span className="grow" />
        <Link className="g-btn secondary" to={`/docs?page=${encodeURIComponent(path)}`}>
          Отмена
        </Link>
        <Button variant="primary" onClick={() => void submit()} busy={save.isPending} disabled={save.isPending || errors.length > 0}>
          <Icon.check size={13} />
          {save.isPending ? "Сохраняю…" : "Сохранить"}
        </Button>
      </footer>

    </div>
  );
}
