import { useState } from "react";
import { useAddArtifact } from "@/entities/task";
import { Icon, Modal, useToast } from "@/shared/ui";

const KINDS: [string, string][] = [
  ["doc", "Документ"],
  ["analysis", "Анализ"],
  ["plan", "План"],
  ["code", "Код"],
  ["log", "Лог"],
  ["other", "Другое"],
];

/** Attach a text artifact (typed in or read from a local text file) to a task or an epic. */
export function AddArtifactDialog({ task, epic, onClose }: { task: string; epic?: boolean; onClose: () => void }) {
  const add = useAddArtifact();
  const toast = useToast();
  const [name, setName] = useState("");
  const [kind, setKind] = useState("doc");
  const [text, setText] = useState("");
  const [note, setNote] = useState("");

  const readFile = (file: File | undefined) => {
    if (!file) return;
    if (file.size > 900_000) return toast("Файл слишком большой для загрузки из браузера", "error");
    file.text().then((t) => {
      setText(t);
      if (!name) setName(file.name);
    });
  };

  const submit = () => {
    if (!name.trim() || !text) return;
    add.mutate(
      { id: task, name: name.trim(), kind, text, note: note.trim() || undefined },
      {
        onSuccess: () => {
          toast(`${name.trim()} добавлен в ${task}`);
          onClose();
        },
        onError: (e) => toast(`Не добавлено: ${e.message}`, "error"),
      },
    );
  };

  return (
    <Modal label="Новый артефакт" onClose={onClose}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
        onKeyDown={(e) => e.key === "Enter" && (e.metaKey || e.ctrlKey) && submit()}
        className="dlg-form"
      >
        <div className="mh">
          <Icon.file />
          {epic ? `Общий артефакт эпика ${task}` : `Артефакт задачи ${task}`}
          <span className="grow" />
          <button type="button" className="g-icon-btn" onClick={onClose} aria-label="Закрыть">
            <Icon.close />
          </button>
        </div>
        <div className="mb">
          <div className="opts">
            <input aria-label="Имя файла" placeholder="имя, например glossary.md" required value={name} onChange={(e) => setName(e.target.value)} className="grow-input" />
            <select aria-label="Вид" value={kind} onChange={(e) => setKind(e.target.value)}>
              {KINDS.map(([v, n]) => (
                <option key={v} value={v}>
                  {n}
                </option>
              ))}
            </select>
            <label className="g-btn sm secondary">
              Из файла…
              <input type="file" hidden onChange={(e) => readFile(e.target.files?.[0])} />
            </label>
          </div>
          <label className="g-field">
            <span className="g-label">Содержимое</span>
            <textarea className="mono-area" rows={12} value={text} onChange={(e) => setText(e.target.value)} placeholder="Markdown, текст, CSV…" />
          </label>
          <label className="g-field">
            <span className="g-label">Пояснение (необязательно)</span>
            <input className="g-input" value={note} onChange={(e) => setNote(e.target.value)} placeholder="зачем он командам" />
          </label>
        </div>
        <div className="mf">
          {epic ? "Команды всех задач эпика увидят его в своей задаче" : "Команда задачи сможет прочитать его"}
          <span className="grow" />
          <button type="button" className="g-btn ghost" onClick={onClose}>
            Отмена
          </button>
          <button type="submit" className="g-btn primary" disabled={!name.trim() || !text || add.isPending}>
            Добавить
          </button>
        </div>
      </form>
    </Modal>
  );
}
