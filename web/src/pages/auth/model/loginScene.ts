import { displayName } from "@/entities/member";

export type ScenePoint = { x: number; y: number };
export type SceneTeam = { id: number; position: ScenePoint; agents: string[] };
export type SceneTask = { id: number; position: ScenePoint; manager: string; teams: SceneTeam[] };
export type LoginScene = { people: string[]; projectManager: string; tasks: SceneTask[]; roster: string };

export const sceneSize = { width: 900, height: 700 };
export const peoplePositions: ScenePoint[] = [280, 350, 420].map((y) => ({ x: 65, y }));
export const projectPosition: ScenePoint = { x: 245, y: 350 };
const taskPositions: ScenePoint[] = [86, 262, 438, 614].map((y) => ({ x: 445, y }));

// Public demo names mirror the default role pools in genie-core/src/team.rs.
// The login illustration does not request an authenticated user's configuration.
const agentNames = [
  "sherlock", "poirot", "marple", "columbo", "scully", "mulder", "watson", "clouseau",
  "bender", "baymax", "walle", "optimus", "johnny5", "r2d2", "tars", "robocop",
  "gandalf", "yoda", "hermione", "spock", "galadriel", "dumbledore", "morpheus", "picard",
  "murphy", "gremlin", "loki", "jinx", "chaos", "moriarty",
];
const humanNames = ["Алексей", "Мария", "Даниил", "София", "Илья", "Вера", "Никита", "Анна", "Марк", "Лена", "Миша", "Саша"];

function shuffled<T>(items: readonly T[]): T[] {
  const result = [...items];
  for (let index = result.length - 1; index > 0; index--) {
    const other = Math.floor(Math.random() * (index + 1));
    [result[index], result[other]] = [result[other], result[index]];
  }
  return result;
}

export function createLoginScene(previousRoster: string | null): LoginScene {
  const people = shuffled(humanNames).slice(0, 3);
  const agents = shuffled(agentNames).map(displayName);
  const roster = () => [...people, ...agents].join("|");
  if (roster() === previousRoster) [agents[0], agents[1]] = [agents[1], agents[0]];
  return {
    people,
    projectManager: agents[0],
    tasks: taskPositions.map((position, index) => ({
      id: index + 1,
      position,
      manager: agents[index + 1],
      teams: [-40, 40].map((offset, teamIndex) => {
        const id = index * 2 + teamIndex;
        return {
          id: id + 1,
          position: { x: 745, y: position.y + offset },
          agents: agents.slice(5 + id * 3, 8 + id * 3),
        };
      }),
    })),
    roster: roster(),
  };
}

function route(from: ScenePoint, to: ScenePoint): string {
  const middle = (from.x + to.x) / 2;
  return `M${from.x} ${from.y} C${middle} ${from.y} ${middle} ${to.y} ${to.x} ${to.y}`;
}

export const sceneRoutes = [
  ...peoplePositions.map((person) => route({ x: person.x + 55, y: person.y }, { x: projectPosition.x - 80, y: projectPosition.y })),
  ...taskPositions.map((task) => route({ x: projectPosition.x + 80, y: projectPosition.y }, { x: task.x - 75, y: task.y })),
  ...taskPositions.flatMap((task) => [-40, 40].map((offset) => route({ x: task.x + 75, y: task.y }, { x: 642, y: task.y + offset }))),
];

export function scenePosition(point: ScenePoint): { left: string; top: string } {
  return { left: `${point.x / sceneSize.width * 100}%`, top: `${point.y / sceneSize.height * 100}%` };
}
