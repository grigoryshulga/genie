import { createContext, type ReactNode, useCallback, useContext, useState } from "react";
import { Icon } from "./icons.tsx";

type Toast = { id: number; text: ReactNode; kind: "ok" | "error" };
const ToastCtx = createContext<(text: ReactNode, kind?: "ok" | "error") => void>(() => {});
export const useToast = () => useContext(ToastCtx);

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toast, setToast] = useState<Toast | undefined>();
  const show = useCallback((text: ReactNode, kind: "ok" | "error" = "ok") => {
    const id = Date.now();
    setToast({ id, text, kind });
    setTimeout(() => setToast((t) => (t?.id === id ? undefined : t)), kind === "error" ? 5000 : 2600);
  }, []);
  return (
    <ToastCtx.Provider value={show}>
      {children}
      {/* One polite live region for the whole app; it stays mounted so screen readers pick up the text. */}
      <div className="g-toast-region" role="status" aria-live="polite">
        {toast && (
          <div className={`g-toast${toast.kind === "error" ? " danger" : ""}`} key={toast.id}>
            {toast.kind === "ok" && <Icon.check size={13} />}
            {toast.text}
          </div>
        )}
      </div>
    </ToastCtx.Provider>
  );
}
