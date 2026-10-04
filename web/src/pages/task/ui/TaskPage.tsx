import { useNavigate, useParams } from "react-router";
import { useTasks } from "@/entities/task";
import { useTeamMap } from "@/entities/team";
import { TeamView } from "@/pages/team";
import { TaskDetail, type TaskTab } from "@/widgets/task-detail";

/** `/task/:taskId/:tab?`: a task on its own page, with its description and its team as tabs. */
export function TaskPage() {
  const { taskId = "", tab } = useParams();
  const navigate = useNavigate();
  const teams = useTeamMap();
  const summary = useTasks().data?.find((t) => t.id === taskId);
  const team = summary?.team ? teams.get(summary.team) : undefined;
  const current: TaskTab = tab === "team" ? "team" : "about";
  return (
    <TaskDetail
      key={taskId}
      id={taskId}
      variant="page"
      tab={current}
      team={team}
      teamPane={team && current === "team" ? <TeamView teamId={team.id} /> : undefined}
      onClose={() => navigate("/tasks")}
    />
  );
}
