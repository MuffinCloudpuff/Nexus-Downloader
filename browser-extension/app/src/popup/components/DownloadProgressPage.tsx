import {Caption1, makeStyles} from "@fluentui/react-components";
import {useEffect, useRef} from "react";
import type {DesktopConnectionState, GenericTaskSummary, TaskAction} from "../../shared/types";
import {TaskCard} from "./TaskCard";

const useStyles = makeStyles({
  root: {display: "flex", flexDirection: "column", gap: "6px", padding: "6px"},
});

export function DownloadProgressPage({taskId, tasks, connectionState, connectionMessage, busy, onAction}: {
  taskId: string;
  tasks: GenericTaskSummary[];
  connectionState: DesktopConnectionState;
  connectionMessage: string;
  busy: boolean;
  onAction: (action: TaskAction) => void;
}) {
  const styles = useStyles();
  // Never fall back to the latest/all tasks, including after deletion or reload.
  const task = tasks.find(item => item.taskId === taskId);
  const connected = connectionState === "connected";
  const taskPresent = Boolean(task);
  const observedTask = useRef({taskId, seen: false});
  useEffect(() => {
    if (observedTask.current.taskId !== taskId) observedTask.current = {taskId, seen: false};
    if (taskPresent) observedTask.current.seen = true;
    else if (connected && observedTask.current.seen) {
      // A known task disappearing from a connected snapshot means it was
      // deleted elsewhere. Initial loading/unknown IDs are not deletions.
      observedTask.current.seen = false;
      window.close();
    }
  }, [taskId, taskPresent, connected]);
  useEffect(() => {
    if (task?.status !== "completed") return;
    // Briefly show completion, then dismiss only this automatic task panel.
    // Switching tasks or leaving completion must cancel the previous close.
    const timer = window.setTimeout(() => window.close(), 800);
    return () => window.clearTimeout(timer);
  }, [taskId, task?.status]);
  return <section className={styles.root} aria-label="本次下载">
    {task && !connected ? <Caption1 role="status">{connectionMessage || "正在连接后台…"}</Caption1> : null}
    {task ? <TaskCard task={task} busy={busy || !connected} onAction={onAction} />
      : <Caption1 role="status">{connected ? "此任务已删除或不再可用" : connectionMessage || "正在等待此下载的进度…"}</Caption1>}
  </section>;
}
