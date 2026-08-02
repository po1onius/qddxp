import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { AlertCircle, CheckCircle2, Info, X } from 'lucide-react';

type ToastType = 'success' | 'error' | 'warning' | 'info';

type ToastInput = {
  message: string;
  type?: ToastType;
  durationMs?: number;
};

type ToastItem = Required<Omit<ToastInput, 'durationMs'>> & {
  id: string;
};

type ToastContextValue = {
  showToast: (toast: ToastInput) => void;
};

const ToastContext = createContext<ToastContextValue | null>(null);
const DEFAULT_TOAST_DURATION_MS = 4000;

const toastStyles: Record<ToastType, { className: string; icon: ReactNode }> = {
  success: {
    className: 'border-emerald-200 bg-emerald-50 text-emerald-900',
    icon: <CheckCircle2 className="mt-0.5 shrink-0 text-emerald-600" size={18} />,
  },
  error: {
    className: 'border-red-200 bg-red-50 text-red-900',
    icon: <AlertCircle className="mt-0.5 shrink-0 text-red-600" size={18} />,
  },
  warning: {
    className: 'border-amber-200 bg-amber-50 text-amber-950',
    icon: <AlertCircle className="mt-0.5 shrink-0 text-amber-600" size={18} />,
  },
  info: {
    className: 'border-slate-200 bg-white text-slate-900',
    icon: <Info className="mt-0.5 shrink-0 text-slate-600" size={18} />,
  },
};

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const nextId = useRef(0);
  const timers = useRef(new Map<string, number>());
  const lastToast = useRef<{ message: string; time: number; type: ToastType } | null>(null);

  const dismissToast = useCallback((id: string) => {
    const timer = timers.current.get(id);
    if (timer) {
      window.clearTimeout(timer);
      timers.current.delete(id);
    }
    setToasts((current) => current.filter((toast) => toast.id !== id));
  }, []);

  const showToast = useCallback(
    ({ durationMs = DEFAULT_TOAST_DURATION_MS, message, type = 'info' }: ToastInput) => {
      const trimmedMessage = message.trim();
      if (!trimmedMessage) {
        return;
      }

      const now = Date.now();
      if (lastToast.current?.message === trimmedMessage && lastToast.current.type === type && now - lastToast.current.time < 500) {
        return;
      }
      lastToast.current = { message: trimmedMessage, time: now, type };

      const id = `${now}-${nextId.current}`;
      nextId.current += 1;
      setToasts((current) => [...current, { id, message: trimmedMessage, type }].slice(-4));

      const timer = window.setTimeout(() => dismissToast(id), durationMs);
      timers.current.set(id, timer);
    },
    [dismissToast],
  );

  useEffect(() => {
    return () => {
      for (const timer of timers.current.values()) {
        window.clearTimeout(timer);
      }
      timers.current.clear();
    };
  }, []);

  const value = useMemo(() => ({ showToast }), [showToast]);

  return (
    <ToastContext.Provider value={value}>
      {children}
      <div
        aria-live="polite"
        className="pointer-events-none fixed right-4 top-4 z-50 flex w-[calc(100vw-2rem)] max-w-sm flex-col gap-3 sm:right-6 sm:top-6"
      >
        {toasts.map((toast) => {
          const style = toastStyles[toast.type];
          return (
            <div
              className={`pointer-events-auto flex items-start gap-3 rounded-md border px-4 py-3 text-sm shadow-panel ${style.className}`}
              key={toast.id}
              role={toast.type === 'error' ? 'alert' : 'status'}
            >
              {style.icon}
              <p className="min-w-0 flex-1 break-words leading-5">{toast.message}</p>
              <button
                aria-label="关闭通知"
                className="rounded p-0.5 text-current opacity-60 hover:bg-black/5 hover:opacity-100"
                onClick={() => dismissToast(toast.id)}
                type="button"
              >
                <X size={16} />
              </button>
            </div>
          );
        })}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast() {
  const context = useContext(ToastContext);
  if (!context) {
    throw new Error('useToast must be used within ToastProvider');
  }
  return context;
}
