import { useEffect, useState } from "react";
import { createLoginScene, peoplePositions, projectPosition, scenePosition, sceneRoutes, sceneSize } from "../model/loginScene";

const rosterStorageKey = "genie-login-roster";

export function LoginBackdrop() {
  const [scene] = useState(() => {
    let previousRoster: string | null = null;
    try {
      previousRoster = window.sessionStorage.getItem(rosterStorageKey);
    } catch {
      // The illustration also works when browser storage is disabled.
    }
    return createLoginScene(previousRoster);
  });

  useEffect(() => {
    try {
      window.sessionStorage.setItem(rosterStorageKey, scene.roster);
    } catch {
      // Random names do not depend on browser storage being available.
    }
  }, [scene]);

  return (
    <div className="login-universe" aria-hidden="true">
      <div className="login-nebula login-nebula-a" />
      <div className="login-nebula login-nebula-b" />
      <div className="login-galaxy" />
      <div className="login-stars" />
      <div className="login-star-dust" />
      <div className="login-star-flare login-star-flare-a" />
      <div className="login-star-flare login-star-flare-b" />
      <div className="login-shooting-star" />
      <div className="login-constellation">
        <svg className="login-network" viewBox={`0 0 ${sceneSize.width} ${sceneSize.height}`} preserveAspectRatio="none">
          <defs>
            <radialGradient id="login-hub-glow">
              <stop stopColor="#ab91ef" stopOpacity=".45" />
              <stop offset="1" stopColor="#6c54ae" stopOpacity="0" />
            </radialGradient>
          </defs>
          <g className="login-network-orbits">
            <ellipse cx="245" cy="350" rx="170" ry="255" transform="rotate(-24 245 350)" />
            <ellipse cx="245" cy="350" rx="215" ry="315" transform="rotate(-24 245 350)" />
          </g>
          <circle className="login-network-halo" cx="245" cy="350" r="150" fill="url(#login-hub-glow)" />
          <g className="login-network-routes">
            {sceneRoutes.map((path, index) => (
              <g key={index}>
                <path d={path} />
                <path className="login-route-echo" d={path} transform="translate(0 4)" />
                <path className="login-route-echo" d={path} transform="translate(0 -4)" />
              </g>
            ))}
          </g>
          <g className="login-network-pulses">
            {sceneRoutes.map((path, index) => <path key={index} d={path} pathLength="1000" />)}
          </g>
          <g className="login-network-ports">
            {scene.tasks.map((task) => (
              <g key={task.id}>
                <circle cx="520" cy={task.position.y} r="2.5" />
                {task.teams.map((team) => <circle key={team.id} cx="642" cy={team.position.y} r="2" />)}
              </g>
            ))}
          </g>
        </svg>

        {scene.people.map((name, index) => (
          <div className="login-person" key={index} style={scenePosition(peoplePositions[index])}>
            <span className="login-person-avatar">{name[0]}</span>
            <span><b>{name}</b><small>человек</small></span>
          </div>
        ))}

        <div className="login-manager login-project-manager" style={scenePosition(projectPosition)}>
          <span className="login-manager-symbol">✧</span>
          <span className="login-manager-copy"><b>{scene.projectManager}</b><small>менеджер проекта</small></span>
          <i className="login-node-status" />
        </div>

        {scene.tasks.map((task) => (
          <div className="login-task-branch" key={task.id}>
            <div className="login-manager login-task-manager" style={scenePosition(task.position)}>
              <span className="login-task-number">0{task.id}</span>
              <span className="login-manager-copy"><b>{task.manager}</b><small>менеджер задачи</small></span>
              <i className="login-node-status" />
            </div>
            {task.teams.map((team) => (
              <div className="login-crew" key={team.id} style={scenePosition(team.position)}>
                <div className="login-crew-head"><span>команда 0{team.id}</span><i>задача 0{task.id}</i></div>
                <div className="login-crew-agents">
                  {team.agents.map((name, index) => (
                    <span className={`login-crew-agent login-crew-agent-${index + 1}`} key={name}>
                      <i>{name[0]}</i><span>{name}</span>
                    </span>
                  ))}
                </div>
              </div>
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}
