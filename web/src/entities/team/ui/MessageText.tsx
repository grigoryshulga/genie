import { useState } from "react";
import { imageRefSrc, parseImageRefs, type ImageRef } from "@/shared/lib";
import { ImagePreview, Markdown } from "@/shared/ui";
import "./message.css";

/**
 * Chat message body: Markdown (headings, lists, tables, code) through the shared
 * safe renderer, and local `!image[...]` references as inline previews between
 * its blocks; an unresolvable reference stays literal text. `team` — the chat's
 * team: file paths are read from its worktree.
 */
export function MessageText({ text, team }: { text: string; team?: string }) {
  return (
    <div className="message-text">
      {parseImageRefs(text).map((segment, i) =>
        segment.type === "text" ? (
          segment.text.trim() && <Markdown key={i} text={segment.text} />
        ) : (
          <ChatImage key={i} imageRef={segment.ref} raw={segment.raw} team={team} />
        ),
      )}
    </div>
  );
}

/** One reference: preview on success, the original token on any failure. */
function ChatImage({ imageRef, raw, team }: { imageRef: ImageRef; raw: string; team?: string }) {
  const [failed, setFailed] = useState(false);
  if (failed) return <p className="message-text-raw">{raw}</p>;
  return (
    <ImagePreview
      src={imageRefSrc(imageRef, team)}
      variant="inline"
      alt={raw}
      caption={imageRef.kind === "path" ? imageRef.path : raw}
      onUnavailable={() => setFailed(true)}
    />
  );
}
