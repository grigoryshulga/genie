import { cloneElement, type InputHTMLAttributes, isValidElement, type ReactElement, type ReactNode, type SelectHTMLAttributes, type TextareaHTMLAttributes, useId } from "react";
import { cx } from "./cx.ts";

/** A label, the control, optional help and error. The control gets its id and aria wiring from here. */
export function Field({ label, help, error, children }: { label: ReactNode; help?: ReactNode; error?: ReactNode; children: ReactElement<Record<string, unknown>> }) {
  const id = useId();
  const describedBy = [help ? `${id}-help` : "", error ? `${id}-error` : ""].filter(Boolean).join(" ") || undefined;
  const control = isValidElement(children) ? cloneElement(children, { id, "aria-describedby": describedBy, "aria-invalid": error ? true : undefined }) : children;
  return (
    <div className="g-field">
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

export function Textarea({ className, ...rest }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea {...rest} className={cx("g-textarea", className)} />;
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
