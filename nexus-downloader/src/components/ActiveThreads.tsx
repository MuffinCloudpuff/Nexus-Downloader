import { useApp } from "../store";
import { useRuntimeMetrics } from "../shell/metrics-store";
import { cn } from "../lib/utils";

export function ActiveThreads() {
  const { settings, t } = useApp();
  const { activeConnections } = useRuntimeMetrics();

  // Show active connections, falling back to 0 when idle.
  const isActive = activeConnections > 0;
  const displayValue = activeConnections;

  return (
    <div className="flex items-center gap-2">
      {isActive ? (
        <span className="relative flex h-2 w-2">
          <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-green-400 opacity-75"></span>
          <span className="relative inline-flex rounded-full h-2 w-2 bg-green-500"></span>
        </span>
      ) : (
        <span className="relative flex h-2 w-2">
          <span className="relative inline-flex rounded-full h-2 w-2 bg-black/20 dark:bg-white/20"></span>
        </span>
      )}
      <span className={cn(
        "text-sm font-black transition-colors duration-300",
        isActive ? "text-green-500" : "text-black/40 dark:text-white/40"
      )}>
        {displayValue} {t.threadCountUnit}
      </span>
    </div>
  );
}
