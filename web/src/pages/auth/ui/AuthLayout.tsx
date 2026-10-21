import type { ReactNode } from "react";
import { Banner, Character, Icon } from "@/shared/ui";

/**
 * The full-bleed brand surface of the pages outside the shell: the slogan with the orange stop on
 * the left, the arcs motif cropped at the corner, and the form in a g-card on the right. The pages
 * share it so signing in, joining and the first project read as one place.
 */
export function AuthLayout({
  slogan,
  pitch,
  title,
  note,
  foot,
  children,
}: {
  /** At most six words; the orange full stop is added here. */
  slogan: string;
  pitch?: ReactNode;
  title: string;
  /** Server state the person must read before the form: why the server is away, why there is no project yet. */
  note?: string;
  foot?: ReactNode;
  children: ReactNode;
}) {
  return (
    <main className="auth-surface">
      <section className="auth-brand">
        <span className="auth-mark" aria-hidden="true">
          <Icon.mark size={24} />
        </span>
        <div className="auth-slogan-block">
          <h1 className="auth-slogan">
            {slogan}
            <span className="g-stop" aria-hidden="true">.</span>
          </h1>
          {pitch && <p className="auth-pitch">{pitch}</p>}
          <Character state="sleep" size={168} />
        </div>
        <Arcs />
      </section>
      <section className="auth-side">
        <div className="g-card auth-card">
          <h2 className="auth-title">{title}</h2>
          {note && <Banner tone="danger">{note}</Banner>}
          {children}
          {foot && <p className="auth-foot">{foot}</p>}
        </div>
      </section>
    </main>
  );
}

/** The arcs motif: concentric quarter rings anchored to a corner and cropped by two edges. Decorative only. */
function Arcs() {
  return (
    <svg className="auth-arcs" viewBox="0 0 500 500" fill="none" aria-hidden="true" focusable="false">
      <g transform="translate(500 500)">
        <path className="auth-arc-outer" d="M-500 0A500 500 0 0 1 0-500V-360A360 360 0 0 0-360 0Z" />
        <path className="auth-arc-inner" d="M-250 0A250 250 0 0 1 0-250V-140A140 140 0 0 0-140 0Z" />
        <path className="auth-arc-core" d="M-105 0A105 105 0 0 1 0-105V0Z" />
      </g>
    </svg>
  );
}
