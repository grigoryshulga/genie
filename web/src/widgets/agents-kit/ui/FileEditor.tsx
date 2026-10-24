// Edit a configuration file as text. The save is refused when the file changed
// since it was opened (409) or when the change is invalid (422): the message
// says what to do, and the lines it (or a known problem) names are marked.

import { useQueryClient } from "@tanstack/react-query";
import { type ReactNode, useMemo, useRef, useState } from "react";
import { agentKeys, type ConfigKind, type Problem, problemLines, refusalMessages, useSaveConfig } from "@/entities/agent-config";
import { ApiError } from "@/shared/api";
import { Dialog, useToast } from "@/shared/ui";

/** A text editor with line numbers; the lines problems name are marked and listed. */
function CodeArea({ value, onChange, marks, label }: { value: string; onChange: (v: string) => void; marks: Map<number, string[]>; label: string }) {
  const area = useRef<HTMLTextAreaElement>(null);
  const gutter = useRef<HTMLDivElement>(null);
  const lines = value.split("\n");
  const go = (line: number) => {
    const el = area.current;
    if (!el) return;
    const start = lines.slice(0, line - 1).reduce((n, l) => n + l.length + 1, 0);
    el.focus();
    el.setSelectionRange(start, start + (lines[line - 1]?.length ?? 0));
    el.scrollTop = Math.max(0, (line - 4) * (parseFloat(getComputedStyle(el).lineHeight) || 18));
  };
  return (
    <>
      <div className="code-area">
        <div className="gutter" ref={gutter} aria-hidden="true">
          {lines.map((_, i) => (
            <div key={i} className={marks.has(i + 1) ? "bad" : undefined}>
              {i + 1}
            </div>
          ))}
        </div>
        <textarea
          ref={area}
          className="g-mono code-edit"
          rows={24}
          wrap="off"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onScroll={(e) => {
            if (gutter.current) gutter.current.scrollTop = e.currentTarget.scrollTop;
          }}
          spellCheck={false}
          aria-label={label}
        />
      </div>
      {marks.size > 0 && (
        <ul className="code-marks" aria-label="Строки с проблемами">
          {[...marks.entries()]
            .sort((a, b) => a[0] - b[0])
            .map(([line, messages]) => (
              <li key={line}>
                <button type="button" onClick={() => go(line)}>
                  строка {line}
                </button>
                <span>{messages.join("; ")}</span>
              </li>
            ))}
        </ul>
      )}
    </>
  );
}

export function FileEditor({
  kind,
  id,
  title,
  hint,
  initial,
  baseHash,
  problems,
  onClose,
  onSaved,
}: {
  kind: ConfigKind;
  id: string;
  title: string;
  hint?: ReactNode;
  initial: string;
  baseHash: string | undefined;
  /** Problems of the item as it is now. */
  problems?: Problem[];
  onClose: () => void;
  onSaved?: () => void;
}) {
  const [text, setText] = useState(initial);
  const [error, setError] = useState<{ text: string; stale: boolean }>();
  const marks = useMemo(() => problemLines(text, [...(problems ?? []).map((p) => p.message), ...(error ? refusalMessages(error.text) : [])]), [text, problems, error]);
  const [busy, setBusy] = useState(false);
  const save = useSaveConfig();
  const qc = useQueryClient();
  const toast = useToast();
  const submit = async () => {
    setBusy(true);
    try {
      const out = await save(kind, id, { content: text }, baseHash);
      const warn = out.problems.filter((p) => p.level === "warning");
      toast(warn.length ? `Сохранено, есть предупреждения: ${warn.map((p) => p.message).join("; ")}` : "Сохранено");
      onSaved?.();
      onClose();
    } catch (e) {
      setError({ text: e instanceof Error ? e.message : String(e), stale: e instanceof ApiError && e.status === 409 });
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog
      title={title}
      onClose={onClose}
      wide
      footer={
        <>
          <span className="ag-save-note">Файл проверяется перед сохранением</span>
          <button type="button" className="g-btn ghost" onClick={onClose}>
            Отмена
          </button>
          <button type="button" className="g-btn primary" disabled={busy} onClick={() => void submit()}>
            Сохранить
          </button>
        </>
      }
    >
      <div className="ag-file-edit">
        {hint && <div className="g-muted ag-hint">{hint}</div>}
        <CodeArea value={text} onChange={setText} marks={marks} label="Содержимое файла" />
        {error && (
          <div className="ag-form-error" role="alert">
            {error.text}
            {error.stale && (
              <>
                {" "}
                <button
                  type="button"
                  className="g-btn secondary"
                  onClick={() => {
                    void qc.invalidateQueries({ queryKey: agentKeys.all });
                    onClose();
                  }}
                >
                  Закрыть и открыть заново
                </button>
              </>
            )}
          </div>
        )}
      </div>
    </Dialog>
  );
}
