import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Link, useNavigate } from "react-router";
import { type Notification, useNotifications } from "@/entities/platform";
import { request } from "@/shared/api";
import { anchorPopover, timeAgo, useTick } from "@/shared/lib";
import { EmptyState, ErrorState, Icon, Skeleton, useRefreshingAct } from "@/shared/ui";
import "./notifications-popover.css";

/** The popup is this wide when there is room for it; the phone turns it into a sheet. */
const WIDTH = 360;
/** The newest notifications the popup lists; the page behind it has the rest. */
const SHOWN = 8;

/**
 * The bell and its popup: the newest notifications with the unread count, the mark-all-read
 * action and the way to the full page. Escape, an outside click or the trigger close it and
 * return focus to the bell; choosing a notification goes where it points. The popup is placed
 * in the window so it never leaves the viewport, and on a phone the stylesheet turns it into a
 * bottom sheet like the filter popovers.
 */
export function NotificationsPopover() {
  useTick();
  const [open, setOpen] = useState(false);
  const host = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const pop = useRef<HTMLDivElement>(null);
  const q = useNotifications();
  const navigate = useNavigate();
  const act = useRefreshingAct();
  const unread = q.data?.unread ?? 0;
  const items = (q.data?.items ?? []).slice(0, SHOWN);

  const close = (restoreFocus = true) => {
    setOpen(false);
    if (restoreFocus) button.current?.focus();
  };

  // The bell sits in the top bar: under it is room enough, but the popup still follows the window
  // when the bar scrolls sideways and stays clear of the edges. The phone sheet ignores these.
  useLayoutEffect(() => {
    if (!open) return;
    const node = pop.current;
    const trigger = button.current;
    if (!node || !trigger) return;
    const place = () => {
      const box = trigger.getBoundingClientRect();
      const { left, top, maxHeight } = anchorPopover({ left: box.left, bottom: box.bottom }, Math.min(WIDTH, node.offsetWidth || WIDTH), {
        width: window.innerWidth,
        height: window.innerHeight,
      });
      node.style.setProperty("--npop-left", `${left}px`);
      node.style.setProperty("--npop-top", `${top}px`);
      node.style.setProperty("--npop-max", `${maxHeight}px`);
    };
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    pop.current?.focus();
    // An outside click closes without pulling focus back: the person's click decides where focus
    // goes. Escape, which has no such intent behind it, returns focus to the bell.
    const onDown = (e: MouseEvent) => {
      if (!host.current?.contains(e.target as Node)) close(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps

  /** Take a notification: it becomes read, and a link in it leads on and closes the popup. */
  const take = (n: Notification) =>
    void act(async () => {
      if (!n.readAt) await request("POST", "/api/notifications/read", { id: n.id });
      if (!n.link) return;
      close(false);
      navigate(n.link);
    });

  const readAll = () => void act(() => request("POST", "/api/notifications/read", {}));

  return (
    <div className="g-popover-host" ref={host}>
      <button
        ref={button}
        type="button"
        className="g-icon-btn g-bell"
        aria-label={unread > 0 ? `Уведомления, непрочитанных: ${unread}` : "Уведомления"}
        title="Уведомления"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? "notifications-popup" : undefined}
        onClick={() => setOpen((was) => !was)}
      >
        <Icon.bell size={16} />
        {unread > 0 && <i className="g-unread" aria-hidden="true" />}
      </button>
      {open && (
        <div id="notifications-popup" ref={pop} className="notif-pop" role="dialog" aria-label="Уведомления" tabIndex={-1}>
          <div className="notif-pop-head">
            <strong>Уведомления</strong>
            {unread > 0 && <span className="g-caption">{unread} непрочитанных</span>}
            {unread > 0 && (
              <button type="button" className="g-btn sm secondary notif-pop-readall" onClick={readAll}>
                Прочитать все
              </button>
            )}
          </div>
          <div className="notif-pop-body">
            {q.isPending ? (
              <div className="notif-pop-list" aria-hidden="true">
                {[0, 1, 2].map((i) => (
                  <div key={i} className="notif-pop-item notif-pop-skeleton">
                    <Skeleton width={14} height={14} />
                    <Skeleton width={`${70 - i * 12}%`} />
                  </div>
                ))}
              </div>
            ) : q.isError ? (
              <ErrorState message={q.error instanceof Error ? q.error.message : String(q.error)} onRetry={() => void q.refetch()} />
            ) : items.length === 0 ? (
              <EmptyState title="Уведомлений нет">Здесь появятся ответы агентов, решения владельцев и упоминания.</EmptyState>
            ) : (
              <div className="notif-pop-list">
                {items.map((n) => (
                  <button key={n.id} type="button" className="notif-pop-item" data-unread={n.readAt ? undefined : "true"} onClick={() => take(n)}>
                    <span className="notif-pop-kind" aria-hidden="true">
                      {kindIcon(n.kind)}
                    </span>
                    <span className="notif-pop-text">
                      <b>{n.title}</b>
                      {n.body && <span className="notif-pop-body-text">{n.body}</span>}
                    </span>
                    <span className="notif-pop-when g-caption">{timeAgo(n.created)}</span>
                    {!n.readAt && <span className="g-sr">не прочитано</span>}
                  </button>
                ))}
              </div>
            )}
          </div>
          <div className="notif-pop-foot">
            <Link to="/notifications" className="g-btn sm secondary" onClick={() => close(false)}>
              Все уведомления
            </Link>
          </div>
        </div>
      )}
    </div>
  );
}

/** The notification's own mark: the same shapes the notifications page uses for its rows. */
function kindIcon(kind: string) {
  if (kind.startsWith("task.comment")) return <Icon.send size={14} />;
  if (kind.startsWith("task.merge") || kind.startsWith("task.pr")) return <Icon.merge size={14} />;
  if (kind.startsWith("task.status")) return <Icon.bolt size={14} />;
  if (kind.startsWith("doc")) return <Icon.file size={14} />;
  if (kind.startsWith("agent") || kind.startsWith("team")) return <Icon.user size={14} />;
  if (kind.startsWith("observation")) return <Icon.info size={14} />;
  return <Icon.bell size={14} />;
}
