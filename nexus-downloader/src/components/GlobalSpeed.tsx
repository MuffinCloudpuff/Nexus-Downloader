import { useApp } from "../store";
import { formatBytes } from "./TaskCard";
import { useRuntimeMetrics } from "../shell/metrics-store";

export const useGlobalSpeed = () => {
  const metrics = useRuntimeMetrics();
  return { dl: metrics.dlSpeed, up: metrics.upSpeed };
};

export function GlobalSpeed() {
  const { t } = useApp();
  const speeds = useGlobalSpeed();

  const formatSpeedValues = (bytes: number) => {
    const formatted = formatBytes(bytes);
    const split = formatted.split(' ');
    return { value: split[0], unit: split.length > 1 ? split[1] : '' };
  };

  const dlStr = formatSpeedValues(speeds.dl);

  return (
    <div className="pt-4 border-t border-black/10 dark:border-white/10 shrink-0 flex flex-col justify-end min-h-[62px] overflow-visible">
      <div className="text-3xl leading-none font-black italic text-blue-500 tracking-tighter whitespace-nowrap">
        {dlStr.value}
        <span className="text-lg leading-none font-bold text-blue-500/60 ml-1 italic tracking-normal">
          {dlStr.unit}/s
        </span>
      </div>
    </div>
  );
}
