import { cloneElement, type InputHTMLAttributes, isValidElement, type KeyboardEvent, type PointerEvent, type ReactElement, type ReactNode, type Ref, type SelectHTMLAttributes, type TextareaHTMLAttributes, useId, useRef, useState } from "react";
import { cx } from "./cx.ts";
import "./select.css";
import "./resize.css";

/** A label, the control, optional help and error. The control gets its id and aria wiring from here. */
export function Field({ label, help, error, className, children }: { label: ReactNode; help?: ReactNode; error?: ReactNode; className?: string; children: ReactElement<Record<string, unknown>> }) {
  const id = useId();
  const describedBy = [help ? `${id}-help` : "", error ? `${id}-error` : ""].filter(Boolean).join(" ") || undefined;
  const control = isValidElement(children) ? cloneElement(children, { id, "aria-describedby": describedBy, "aria-invalid": error ? true : undefined }) : children;
  return (
    <div className={cx("g-field", className)}>
      <label className="g-label" htmlFor={id}>
        {label}
      </label>
      {control}
      {help && (
        <span className="g-help" id={`${id}-help`}>
          {help}
        </span>
      )}
      {error && (
        <span className="g-error" id={`${id}-error`} role="alert">
          {error}
        </span>
      )}
    </div>
  );
}

export function Input({ sm, className, ...rest }: InputHTMLAttributes<HTMLInputElement> & { sm?: boolean }) {
  return <input {...rest} className={cx("g-input", sm && "sm", className)} />;
}

export function Select({ sm, className, children, ...rest }: SelectHTMLAttributes<HTMLSelectElement> & { sm?: boolean }) {
  return (
    <select {...rest} className={cx("g-select", sm && "sm", className)}>
      {children}
    </select>
  );
}

export function Textarea({ className, ref, ...rest }: TextareaHTMLAttributes<HTMLTextAreaElement> & { ref?: Ref<HTMLTextAreaElement> }) {
  return <textarea {...rest} ref={ref} className={cx("g-textarea", className)} />;
}

const RESIZE_MIN = 72;
const RESIZE_MAX = 480;
const RESIZE_STEP = 16;

/** A text area whose height the grip sets, by drag or with the arrow keys; it starts at its rows. */
export function ResizableTextarea({ className, ...rest }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  const field = useRef<HTMLTextAreaElement>(null);
  const [height, setHeight] = useState<number>();
  const [dragging, setDragging] = useState(false);
  const drag = useRef<{ y: number; height: number } | null>(null);
  const clamp = (value: number) => Math.min(RESIZE_MAX, Math.max(RESIZE_MIN, value));
  const current = () => field.current?.offsetHeight ?? RESIZE_MIN;
  const onPointerDown = (event: PointerEvent<HTMLSpanElement>) => {
    drag.current = { y: event.clientY, height: current() };
    event.currentTarget.setPointerCapture(event.pointerId);
    setDragging(true);
  };
  const onPointerMove = (event: PointerEvent<HTMLSpanElement>) => {
    if (drag.current) setHeight(clamp(drag.current.height + event.clientY - drag.current.y));
  };
  const onPointerUp = () => {
    drag.current = null;
    setDragging(false);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLSpanElement>) => {
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
    event.preventDefault();
    setHeight(clamp(current() + (event.key === "ArrowDown" ? RESIZE_STEP : -RESIZE_STEP)));
  };
  return (
    <div className="g-resize">
      <textarea {...rest} ref={field} style={height === undefined ? undefined : { height }} className={cx("g-textarea g-resize-field", className)} />
      <span
        role="separator"
        aria-orientation="horizontal"
        aria-label="Изменить высоту поля"
        title="Потяните, чтобы изменить высоту"
        tabIndex={0}
        className={cx("g-resize-grip", dragging && "dragging")}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onKeyDown={onKeyDown}
      />
    </div>
  );
}

export function Checkbox({ label, className, ...rest }: Omit<InputHTMLAttributes<HTMLInputElement>, "type"> & { label: ReactNode }) {
  return (
    <label className={cx("g-check", className)}>
      <input {...rest} type="checkbox" />
      <span>{label}</span>
    </label>
  );
}

export function Radio({ label, className, ...rest }: Omit<InputHTMLAttributes<HTMLInputElement>, "type"> & { label: ReactNode }) {
  return (
    <label className={cx("g-radio", className)}>
      <input {...rest} type="radio" />
      <span>{label}</span>
    </label>
  );
}

/** A switch is a checkbox with role="switch": native keyboard and form behaviour. */
export function Switch({ label, ...rest }: Omit<InputHTMLAttributes<HTMLInputElement>, "type"> & { label: string }) {
  return (
    <span className="g-switch">
      <input {...rest} type="checkbox" role="switch" aria-label={label} />
    </span>
  );
}

export function SearchField({ label, className, ...rest }: Omit<InputHTMLAttributes<HTMLInputElement>, "type"> & { label: string }) {
  return (
    <label className={cx("g-search", className)}>
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" aria-hidden="true">
        <circle cx="7" cy="7" r="4.5" />
        <path d="M10.5 10.5L14 14" />
      </svg>
      <input {...rest} type="search" aria-label={label} />
    </label>
  );
}
