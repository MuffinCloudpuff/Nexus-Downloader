import { useEffect, useRef } from "react";
import { getTasks, requestPairing, streamAllTaskEvents } from "../api/daemon-client";
import { notifyDownloadCompleted } from "../platform/desktop-shell";
import { useApp } from "../store";

const SNAPSHOT_INTERVAL_MS = 5000;
const STREAM_RECONNECT_MS = 1500;

export function DaemonTaskSync() {
  const { setConnection, replaceTaskSnapshot, applyDaemonTaskEvent } = useApp();
  const cursorsRef = useRef(new Map<string, number>());
  const taskNamesRef = useRef(new Map<string, string>());

  useEffect(() => {
    let cancelled = false;
    let snapshotTimer: number | undefined;

    async function fetchSnapshot() {
      try {
        setConnection({ state: "connecting", message: "Connecting to daemon" });
        const snapshot = await getTasks();
        if (cancelled) {
          return;
        }
        replaceTaskSnapshot(snapshot);
        for (const task of snapshot) {
          taskNamesRef.current.set(task.task_id, task.title || task.output_path || task.task_id);
          cursorsRef.current.set(
            task.task_id,
            Math.max(cursorsRef.current.get(task.task_id) ?? 0, Number(task.event_cursor ?? 0)),
          );
        }
        setConnection({ state: "connected", message: "Daemon connected" });
      } catch (error) {
        if (cancelled) {
          return;
        }
        const message = error instanceof Error ? error.message : "Daemon disconnected";
        if (message.includes("401")) {
          try {
            await requestPairing();
            if (!cancelled) {
              await fetchSnapshot();
            }
            return;
          } catch {
            setConnection({ state: "unauthorized", message: "Daemon pairing required" });
            return;
          }
        }
        setConnection({ state: "disconnected", message });
      }
    }

    void fetchSnapshot();
    snapshotTimer = window.setInterval(() => void fetchSnapshot(), SNAPSHOT_INTERVAL_MS);

    return () => {
      cancelled = true;
      if (snapshotTimer !== undefined) {
        window.clearInterval(snapshotTimer);
      }
    };
  }, [replaceTaskSnapshot, setConnection]);

  useEffect(() => {
    let cancelled = false;
    let reconnectTimer: number | undefined;
    let controller: AbortController | undefined;

    function startDaemonStream() {
      if (cancelled) {
        return;
      }
      controller = new AbortController();
      const streamController = controller;
      void streamAllTaskEvents({
        signal: streamController.signal,
        onEvent: (event) => {
          if (streamController.signal.aborted) {
            return;
          }
          applyDaemonTaskEvent(event);
          taskNamesRef.current.set(event.task_id, taskNamesRef.current.get(event.task_id) || event.task_id);
          if (event.type === "task_completed") {
            void notifyDownloadCompleted(taskNamesRef.current.get(event.task_id) || event.task_id);
          }
          cursorsRef.current.set(
            event.task_id,
            Math.max(cursorsRef.current.get(event.task_id) ?? 0, event.event_id),
          );
        },
      })
        .then(() => {
          scheduleReconnect(streamController);
        })
        .catch(() => {
          scheduleReconnect(streamController);
        });
    }

    function scheduleReconnect(streamController: AbortController) {
      if (cancelled || streamController.signal.aborted) {
        return;
      }
      reconnectTimer = window.setTimeout(() => {
        startDaemonStream();
      }, STREAM_RECONNECT_MS);
    }

    startDaemonStream();

    return () => {
      cancelled = true;
      controller?.abort();
      if (reconnectTimer !== undefined) {
        window.clearTimeout(reconnectTimer);
      }
    };
  }, [applyDaemonTaskEvent]);

  return null;
}
