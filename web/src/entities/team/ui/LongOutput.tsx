import { useState } from "react";
import { Button, Prose } from "@/shared/ui";
import { clipText } from "../clip.ts";

/** A tool's output as a code block; past a dozen lines it is clipped behind «Показать ещё». */
export function LongOutput({ text }: { text: string }) {
  const [all, setAll] = useState(false);
  const clip = clipText(text);
  const long = clip.hidden > 0;
  return (
    <div className="tv-out">
      <Prose>
        <pre>
          <code>{all || !long ? text : clip.head}</code>
        </pre>
      </Prose>
      {long && (
        <Button variant="ghost" sm aria-expanded={all} onClick={() => setAll(!all)}>
          {all ? "Свернуть" : `Показать ещё · ${clip.hidden} стр.`}
        </Button>
      )}
    </div>
  );
}
