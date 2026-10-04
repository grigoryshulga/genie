import { displayName } from "@/entities/member";
import { request, teamKeys, useInvalidating } from "@/shared/api";
import { Icon, useToast } from "@/shared/ui";

/**
 * Restart a member's session: its conversation goes on, the role's current
 * prompt, model, skills and MCP connections take effect (and a member in
 * error works again).
 */
export function RestartMemberButton({ team, name }: { team: string; name: string }) {
  const toast = useToast();
  const restart = useInvalidating(
    () => request("POST", `/api/teams/${encodeURIComponent(team)}/members/${encodeURIComponent(name)}/restart`, {}),
    teamKeys(),
  );
  return (
    <button
      type="button"
      className="icon-btn member-remove"
      title="Перезапустить с текущей ролью — разговор продолжится"
      aria-label={`Перезапустить ${name}`}
      disabled={restart.isPending}
      onClick={() =>
        restart.mutate(undefined, {
          onSuccess: () => toast(`${displayName(name)} перезапускается: разговор продолжится с текущими настройками роли`),
          onError: (e) => toast(`${displayName(name)} не перезапущен: ${e.message}`, "error"),
        })
      }
    >
      <Icon.restart size={12} />
    </button>
  );
}
