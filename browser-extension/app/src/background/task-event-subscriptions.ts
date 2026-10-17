import type {GenericTaskSummary} from "../shared/types";
import type {createDaemonClient} from "./daemon-client";
import type {createTaskStore} from "./task-store";

type DaemonClient = ReturnType<typeof createDaemonClient>;
type TaskStore = ReturnType<typeof createTaskStore>;

const INITIAL_RETRY_DELAY_MS = 1000;
const MAX_RETRY_DELAY_MS = 15000;

export function createTaskEventSubscriptions({
  client,
  taskStore,
  onTasksChanged,
  onTaskUpdated,
}: {
  client: DaemonClient;
  taskStore: TaskStore;
  onTasksChanged: (tasks: GenericTaskSummary[]) => void;
  onTaskUpdated?: (task: GenericTaskSummary | null) => void;
}) {
  let controller: AbortController | null = null;
  let retryTimer: number | null = null;
  let retryDelayMs = INITIAL_RETRY_DELAY_MS;

  function clearRetry(): void {
    if (retryTimer !== null) {
      self.clearTimeout(retryTimer);
      retryTimer = null;
    }
  }

  function stopAll() {
    clearRetry();
    controller?.abort();
    controller = null;
  }

  function scheduleRetry(): void {
    clearRetry();
    retryTimer = self.setTimeout(() => {
      retryTimer = null;
      start();
    }, retryDelayMs);
    retryDelayMs = Math.min(retryDelayMs * 2, MAX_RETRY_DELAY_MS);
  }

  function start() {
    if (controller && !controller.signal.aborted) {
      return;
    }
    stopAll();
    if (!client.hasToken()) {
      return;
    }

    const streamController = new AbortController();
    controller = streamController;
    void client
      .streamAllTaskEvents(streamController.signal, (event) => {
        const updated = taskStore.applyEvent(event);
        retryDelayMs = INITIAL_RETRY_DELAY_MS;
        onTaskUpdated?.(updated);
        onTasksChanged(taskStore.getTasks());
      })
      .then(() => {
        if (streamController.signal.aborted) {
          return;
        }
        if (controller === streamController) {
          controller = null;
        }
        scheduleRetry();
      })
      .catch((error) => {
        if (streamController.signal.aborted) {
          return;
        }
        if (controller === streamController) {
          controller = null;
        }
        console.warn("[GD3] global task event stream disconnected", {
          message: error instanceof Error ? error.message : String(error),
        });
        scheduleRetry();
      });
  }

  function subscribeTask(..._args: unknown[]): void {
    start();
  }

  function stopTask(..._args: unknown[]): void {
    // Global stream is shared by all tasks; individual stop is intentionally a no-op.
  }

  function syncWithActiveTasks(): void {
    start();
  }

  return {
    stopAll,
    stopTask,
    subscribeTask,
    syncWithActiveTasks,
  };
}
