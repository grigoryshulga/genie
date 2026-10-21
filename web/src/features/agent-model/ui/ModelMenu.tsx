import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { displayName, ROLE_TITLE_RU } from "@/entities/member";
import { useMeta } from "@/entities/project";
import { useSetMemberModel } from "@/entities/team";
import { useToast } from "@/shared/ui";
import { ModelList } from "./ModelList.tsx";
import "./model-menu.css";

const short = (id: string) => id.replace(/^[^/]+\//, "");
const capitalized = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);
/** Thinking levels offered in the menu; `""` is the role's. */
const THINKING = ["", "off", "low", "medium", "high", "xhigh"];
/** Where the menu opens: under the button or over it, and how tall it may grow there. */
type Pos = { top?: number; bottom?: number; right: number; maxHeight: number };
const MARGIN = 12;
/** Room below that is enough for the menu to open downwards. */
const ROOMY = 480;
/** The menu never grows taller than this: a long list scrolls. */
const TALLEST = 640;

/**
 * The model of one agent: the button shows what it runs on (its own choice or
 * its role's); the menu picks another model and thinking level for this agent
 * alone. The agent keeps its conversation and switches after its current step.
 */
export function ModelMenu({ team, member }: { team: string; member: { name: string; role: string; model?: string; thinking?: string } }) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<Pos>();
  const btn = useRef<HTMLButtonElement>(null);
  const meta = useMeta().data;
  const ofRole = meta?.roleModels?.[member.role];
  const own = !!(member.model || member.thinking);
  const model = member.model ?? ofRole?.model;
  const thinking = member.thinking ?? ofRole?.thinking;

  useLayoutEffect(() => {
    if (!open || !btn.current) return;
    // Below the button while the menu fits there, else above it; the list scrolls inside whatever height is left.
    const place = () => {
      const r = btn.current!.getBoundingClientRect();
      const right = Math.max(8, window.innerWidth - r.right);
      const below = window.innerHeight - r.bottom - 6 - MARGIN;
      const above = r.top - 6 - MARGIN;
      setPos(below >= Math.min(ROOMY, above) ? { top: r.bottom + 6, right, maxHeight: Math.min(TALLEST, below) } : { bottom: window.innerHeight - r.top + 6, right, maxHeight: Math.min(TALLEST, above) });
    };
    place();
    window.addEventListener("resize", place);
    return () => window.removeEventListener("resize", place);
  }, [open]);

  return (
    <>
      <button ref={btn} type="button" className={`mm-trigger${open ? " open" : ""}`} aria-haspopup="dialog" aria-expanded={open} onClick={() => setOpen(!open)} title={`${model ?? "модель pi по умолчанию"}${thinking ? ` · ${thinking}` : ""}: сменить модель этого агента`}>
        <span className="mono">{model ? `${short(model)}${thinking ? ` · ${thinking}` : ""}` : "по умолчанию"}</span>
        <span className={`mm-src${own ? " own" : ""}`}>{own ? "своя" : "роль"}</span>
        <svg width="10" height="10" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
          <path d="M4 6l4 4 4-4" />
        </svg>
      </button>
      {open && pos && (
        <Popover
          team={team}
          member={member}
          roleModel={ofRole?.model}
          roleThinking={ofRole?.thinking}
          pos={pos}
          onClose={() => {
            setOpen(false);
            btn.current?.focus();
          }}
        />
      )}
    </>
  );
}

function Popover({
  team,
  member,
  roleModel,
  roleThinking,
  pos,
  onClose,
}: {
  team: string;
  member: { name: string; role: string; model?: string; thinking?: string };
  roleModel?: string;
  roleThinking?: string;
  pos: Pos;
  onClose: () => void;
}) {
  const save = useSetMemberModel();
  const toast = useToast();
  const [pick, setPick] = useState(member.model ?? "");
  const [think, setThink] = useState(member.thinking ?? "");
  const who = displayName(member.name);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const changed = pick !== (member.model ?? "") || think !== (member.thinking ?? "");
  const apply = () =>
    save.mutate(
      { team, member: member.name, model: pick || undefined, thinking: think || undefined },
      {
        onSuccess: () => {
          const now = pick || roleModel;
          toast(pick || think ? `${who} перейдёт на ${now ? short(now) : "модель роли"}${think ? ` · ${think}` : ""} со следующего шага` : `${who} вернётся к модели роли со следующего шага`);
          onClose();
        },
        onError: (e) => toast(`Модель не сменилась: ${e.message}`, "error"),
      },
    );

  return (
    <>
      <div className="mm-scrim" onMouseDown={onClose} />
      <div className="mm-pop" role="dialog" aria-label={`Модель ${who}`} style={pos}>
        <div className="mm-hd">
          <b>Модель {who}</b>
          <span>Только для этого агента. Роль и остальная команда не меняются.</span>
        </div>
        <ModelList
          current={member.model}
          pick={pick}
          onPick={(id) => setPick(id === roleModel ? "" : id)}
          note={(m) => (m.id === roleModel ? "у роли" : "")}
          first={
            <button type="button" role="radio" aria-checked={!pick} className={`mm-opt role${!pick ? " on" : ""}`} onClick={() => setPick("")}>
              <span className="mm-radio" />
              <span className="mm-role">
                <span>Как у роли «{capitalized(ROLE_TITLE_RU[member.role] ?? member.role)}»</span>
                <span className="mono muted">
                  {roleModel ? short(roleModel) : "модель pi по умолчанию"}
                  {roleThinking ? ` · ${roleThinking}` : ""}
                </span>
              </span>
            </button>
          }
        />
        <div className="mm-think">
          <span>Размышление</span>
          <div role="radiogroup" aria-label="Размышление" className="mm-seg">
            {THINKING.map((t) => (
              <button key={t || "role"} type="button" role="radio" aria-checked={think === t} className={think === t ? "on" : ""} onClick={() => setThink(t)}>
                {t || "как у роли"}
              </button>
            ))}
          </div>
        </div>
        <div className="mm-ft">
          <span className="muted">{changed ? "Со следующего шага, разговор сохранится" : "Выберите модель"}</span>
          <button type="button" className="g-btn sm secondary" onClick={onClose}>
            Отмена
          </button>
          <button type="button" className="g-btn sm primary" onClick={apply} disabled={!changed || save.isPending}>
            Применить
          </button>
        </div>
      </div>
    </>
  );
}
