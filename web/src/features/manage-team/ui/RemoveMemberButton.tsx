import { useState } from "react";
import { memberLabel } from "@/entities/member";
import { useRemoveMember } from "@/entities/team";
import { ConfirmDialog, Icon, useToast } from "@/shared/ui";

export function RemoveMemberButton({ team, name, role }: { team: string; name: string; role: string }) {
  const [open, setOpen] = useState(false);
  const remove = useRemoveMember();
  const toast = useToast();
  const label = memberLabel(name, role);
  return (
    <>
      <button type="button" className="g-icon-btn member-remove" onClick={() => setOpen(true)} aria-label={`Убрать ${label} из команды`} title="Убрать из команды">
        <Icon.close size={12} />
      </button>
      {open && (
        <ConfirmDialog
          title={`Убрать ${label}?`}
          confirmLabel="Убрать из команды"
          danger
          busy={remove.isPending}
          onClose={() => setOpen(false)}
          onConfirm={() =>
            remove.mutate(
              { team, member: name },
              {
                onSuccess: () => {
                  toast(`${label} убран из команды ${team}`);
                  setOpen(false);
                },
                onError: (e) => toast(`Не удалось: ${e.message}`, "error"),
              },
            )
          }
        >
          Процесс агента будет остановлен, остальные участники получат сообщение, что ждать его не нужно. Его сообщения останутся в чате.
        </ConfirmDialog>
      )}
    </>
  );
}
