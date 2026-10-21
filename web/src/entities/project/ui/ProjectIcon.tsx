import { useState } from "react";
import { projectMonogram } from "../model.ts";
import "./project-icon.css";

/** Decorative project identity beside its visible name, with a fallback for missing images. */
export function ProjectIcon({ name, src, large = false }: { name: string; src?: string; large?: boolean }) {
  const [failed, setFailed] = useState<string>();
  return (
    <span className={`project-icon${large ? " large" : ""}`} aria-hidden="true">
      {src && failed !== src ? <img src={src} alt="" onError={() => setFailed(src)} /> : projectMonogram(name)}
    </span>
  );
}
