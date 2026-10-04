// The decision an agent waits for: its question, the action it attached (the buttons),
// and a free answer that is always there. Each action kind is a card in CARDS; a kind
// this web does not know yet shows as a plain question, so new kinds never break it.

import { type ReactNode, useEffect, useState } from "react";
import { type TaskRepo, useMergeRequest, useRepos, useTaskRepos } from "@/entities/repo";
import { STATUS_NAME, type Status, StatusIcon, type Task, useComment, useMoveTask } from "@/entities/task";
import type { OwnerAction } from "@/shared/api";
import { timeAgo } from "@/shared/lib";
import { ConfirmDialog, Icon, useToast } from "@/shared/ui";
import { answerText, cardKind, type Kind, mergeHeld } from "../model.ts";

type ActionOf<K extends Kind> = Extract<OwnerAction, { kind: K }>;

/** What a card gives the box: a line that goes before the free answer, and whether it alone is an answer. */
interface Picked {
  line?: string;
}

interface CardProps<K extends Kind> {
  task: Task;
  action: ActionOf<K>;
  picked: Picked;
  setPicked: (p: Picked) => void;
  /** The action is done by the card itself (a merge): return the task to work with this note. */
  resolve: (note: string) => void;
}

const HEADER: Record<Kind, string> = {
  "ask-owner-question": "Нужно ваше решение",
  "ask-for-merge-pr": "Нужно ваше решение: слить запрос",
  "ask-free-form": "Нужно ваше решение",
};

function QuestionCard({ action, picked, setPicked }: CardProps<"ask-owner-question">) {
  return (
    <div className="od-options" role="radiogroup" aria-label="Варианты ответа">
      {action.options.map((o) => (
        <button
          key={o}
          type="button"
          role="radio"
          aria-checked={picked.line === o}
          className={picked.line === o ? "on" : ""}
          onClick={() => setPicked(picked.line === o ? {} : { line: o })}
        >
          {o}
        </button>
      ))}
    </div>
  );
}

const CI_TEXT: Record<NonNullable<TaskRepo["ciState"]>, string> = {
  passed: "проверки пройдены",
  failed: "проверки упали",
  pending: "проверки идут",
  stalled: "проверки зависли",
  none: "проверок нет",
};

function MergeCard({ task, action, resolve }: CardProps<"ask-for-merge-pr">) {
  const row = useTaskRepos(task.id).data?.repos.find((r) => r.repo === action.repo);
  const policy = useRepos().data?.find((r) => r.name === action.repo)?.policy?.change_request;
  const merge = useMergeRequest();
  const toast = useToast();
  const [confirm, setConfirm] = useState(false);
  const [refused, setRefused] = useState<string>();
  const number = row?.crNumber ?? action.number;
  const url = row?.crUrl ?? action.url;
  const state = row?.crState ?? "open";
  const ci = row?.ciState ?? "none";
  const requireCi = policy?.require_ci ?? true;
  // The server has the last word (approvals, conflicts); what is known here is said up front.
  const held = mergeHeld(action.repo, state, ci, requireCi);
  const why = held ?? refused;
  return (
    <>
      <div className="od-cr">
        <div className="l1">
          <Icon.merge size={15} />
          <b>{action.repo}</b>
          {number != null && <span className="mono">#{number}</span>}
          {row?.branch && <span className="mono muted">{row.branch}</span>}
        </div>
        <div className="l2">
          <span className={`pill ${ci === "passed" ? "ok" : ci === "failed" ? "fail" : ci === "pending" ? "warn" : ""}`}>{CI_TEXT[ci]}</span>
          <span className="muted">
            Политика: {policy?.method ? `${policy.method}, ` : ""}
            {requireCi ? "нужны зелёные проверки" : "проверки не обязательны"}
            {(policy?.approvals ?? 1) > 0 ? `, одобрений ${policy?.approvals ?? 1} (ваше нажатие считается одним)` : ""}
          </span>
        </div>
      </div>
      {why && (
        <p role="status" className="od-why">
          {why}
        </p>
      )}
      <div className="od-buttons">
        {url && (
          <a className="btn ghost" href={url} target="_blank" rel="noreferrer">
            <Icon.external size={13} /> Посмотреть PR
          </a>
        )}
        <button type="button" className="btn amber" disabled={!!held || merge.isPending} onClick={() => setConfirm(true)}>
          <Icon.merge size={13} /> Слить PR
        </button>
        <span className="hint">После слияния задача вернётся в «{STATUS_NAME[task.needsOwner?.previous ?? "approved"]}»</span>
      </div>
      {confirm && (
        <ConfirmDialog
          title={`Слить запрос${number != null ? ` #${number}` : ""} в ${action.repo}?`}
          confirmLabel="Слить"
          busy={merge.isPending}
          onClose={() => setConfirm(false)}
          onConfirm={() =>
            merge.mutate(
              { task: task.id, repo: action.repo, policy: true },
              {
                onSuccess: () => {
                  setConfirm(false);
                  setRefused(undefined);
                  toast("Запрос слит");
                  resolve(`Owner merged ${action.repo}${number != null ? ` #${number}` : ""}`);
                },
                onError: (e) => {
                  setConfirm(false);
                  setRefused(`Хостинг или политика не пустили: ${e.message}`);
                },
              },
            )
          }
        >
          Запрос сольётся от имени бота genie по политике репозитория{policy?.method ? ` (${policy.method})` : ""}: проверки и одобрения должны быть в порядке, правила защиты веток на хостинге тоже действуют.
        </ConfirmDialog>
      )}
    </>
  );
}

// One card per action kind. A new kind: a variant of OwnerAction on the server, a card here
// and its kind in CARD_KINDS.
const CARDS: { [K in Kind]?: (p: CardProps<K>) => ReactNode } = {
  "ask-owner-question": QuestionCard,
  "ask-for-merge-pr": MergeCard,
};

function Card<K extends Kind>(p: CardProps<K>) {
  const C = CARDS[p.action.kind as K] as ((p: CardProps<K>) => ReactNode) | undefined;
  return C ? <C {...p} /> : null;
}

export function OwnerDecision({ task }: { task: Task }) {
  const toast = useToast();
  const move = useMoveTask();
  const comment = useComment();
  const [words, setWords] = useState("");
  const [picked, setPicked] = useState<Picked>({});
  const owner = task.needsOwner;
  useEffect(() => {
    setWords("");
    setPicked({});
  }, [task.id, owner?.at]);
  if (!owner) return null;
  const action = owner.action;
  const kind = cardKind(action) ?? "ask-free-form";
  const fail = (e: Error) => toast(`Не удалось: ${e.message}`, "error");
  const back = (status: Status, note: string) =>
    move.mutate({ id: task.id, status, note }, { onSuccess: () => toast(`${task.id} → ${STATUS_NAME[status]} · оркестратор уведомлён`), onError: fail });

  const text = answerText(picked.line, words);
  const send = (andReturn: boolean) => {
    if (!text) return;
    comment.mutate(
      { id: task.id, text },
      {
        onSuccess: () => {
          setWords("");
          setPicked({});
          if (andReturn) back(owner.previous, "Owner answered");
          else toast("Ответ отправлен оркестратору");
        },
        onError: fail,
      },
    );
  };
  const hasCard = kind !== "ask-free-form";
  return (
    <section className="owner-box" aria-label="Нужно ваше решение">
      <div className="h">
        <StatusIcon status="needs_owner" size={15} />
        {HEADER[kind]}
        <span className="when">
          {owner.by} · {timeAgo(owner.at)}
        </span>
      </div>
      <p>{owner.question}</p>
      {action && hasCard && <Card task={task} action={action} picked={picked} setPicked={setPicked} resolve={(note) => back(owner.previous, note)} />}
      {hasCard && <span className="od-or">{kind === "ask-owner-question" ? "Уточнение или свой вариант" : "Или ответьте словами"}</span>}
      <textarea
        aria-label="Ваш ответ"
        placeholder={kind === "ask-for-merge-pr" ? "Например: сначала поправь название колонки" : "Ваш ответ уйдёт оркестратору и команде"}
        value={words}
        onChange={(e) => setWords(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && (e.metaKey || e.ctrlKey) && send(true)}
      />
      <div className="actions">
        <span>⌘↵ ответить и вернуть в «{STATUS_NAME[owner.previous]}»</span>
        <span className="grow" />
        <button type="button" className="btn ghost" disabled={!text} onClick={() => send(false)}>
          Только ответить
        </button>
        <button type="button" className={`btn ${kind === "ask-for-merge-pr" ? "ghost" : "amber"}`} disabled={!text} onClick={() => send(true)}>
          Ответить и вернуть в работу
        </button>
      </div>
    </section>
  );
}
