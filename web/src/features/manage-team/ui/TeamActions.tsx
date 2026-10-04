import { useState } from "react";
import { useNavigate } from "react-router";
import { type TeamView, useDeleteTeam, useStopTeam } from "@/entities/team";
import { ConfirmDialog, Icon, useToast } from "@/shared/ui";
import { AddMemberDialog } from "./AddMemberDialog.tsx";

/** Owner controls for a team: add a member, stop the team, delete it. */
export function TeamActions({ team }: { team: TeamView }) {
  const [dialog, setDialog] = useState<"add" | "stop" | "delete" | undefined>();
  const stop = useStopTeam();
  const del = useDeleteTeam();
  const toast = useToast();
  const navigate = useNavigate();
  const active = team.state === "active";
  const close = () => setDialog(undefined);
  const worktreeOption = team.worktree ? `Удалить worktree ${team.worktree.branch} (ветка останется)` : undefined;

  return (
    <>
      {active && (
        <button type="button" className="btn" onClick={() => setDialog("add")}>
          <Icon.userPlus size={13} />
          <span className="d-only">Добавить</span>
        </button>
      )}
      {active && (
        <button type="button" className="btn" onClick={() => setDialog("stop")}>
          <Icon.stop size={13} />
          <span className="d-only">Остановить</span>
        </button>
      )}
      <button type="button" className="icon-btn" onClick={() => setDialog("delete")} aria-label="Удалить команду" title="Удалить команду">
        <Icon.trash />
      </button>

      {dialog === "add" && <AddMemberDialog team={team.id} onClose={close} />}
      {dialog === "stop" && (
        <ConfirmDialog
          title={`Остановить команду ${team.id}?`}
          confirmLabel="Остановить"
          danger
          option={worktreeOption}
          busy={stop.isPending}
          onClose={close}
          onConfirm={(removeWorktree) =>
            stop.mutate(
              { team: team.id, removeWorktree },
              { onSuccess: () => (toast(`Команда ${team.id} остановлена`), close()), onError: (e) => toast(`Не удалось: ${e.message}`, "error") },
            )
          }
        >
          Все агенты команды будут остановлены. Чат и журнал сохранятся; если задача ещё открыта, она освободится для новой команды, а оркестратор получит уведомление.
        </ConfirmDialog>
      )}
      {dialog === "delete" && (
        <ConfirmDialog
          title={`Удалить команду ${team.id}?`}
          confirmLabel="Удалить"
          danger
          option={worktreeOption}
          busy={del.isPending}
          onClose={close}
          onConfirm={(removeWorktree) =>
            del.mutate(
              { team: team.id, removeWorktree },
              {
                onSuccess: () => {
                  toast(`Команда ${team.id} удалена`);
                  close();
                  navigate("/board");
                },
                onError: (e) => toast(`Не удалось: ${e.message}`, "error"),
              },
            )
          }
        >
          Агенты будут остановлены, а команда удалена вместе с чатом и журналом. Задача и её история останутся.
        </ConfirmDialog>
      )}
    </>
  );
}
