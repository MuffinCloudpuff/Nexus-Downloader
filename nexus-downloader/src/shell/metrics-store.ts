import { useEffect, useState } from "react";
import { getRuntimeMetrics } from "../api/daemon-client";
import { useApp } from "../store";

export interface MetricsState {
  dlSpeed: number;
  upSpeed: number;
  activeConnections: number;
}

let metricsState: MetricsState = { dlSpeed: 0, upSpeed: 0, activeConnections: 0 };
const listeners = new Set<(state: MetricsState) => void>();

let pollingInterval: number | null = null;
let activeTaskCount = 0;

function notifyListeners() {
  listeners.forEach(listener => listener(metricsState));
}

async function fetchMetrics() {
  try {
    const metrics = await getRuntimeMetrics();
    metricsState = {
      dlSpeed: metrics.total_speed_bytes_per_sec,
      upSpeed: 0,
      activeConnections: metrics.active_chunk_connections ?? 0,
    };
    notifyListeners();
  } catch (e) {
    // Keep last known state on error, or zero out if preferred.
  }
}

export function useRuntimeMetrics() {
  const { tasks } = useApp();
  const [state, setState] = useState<MetricsState>(metricsState);

  // Sync active task count from the app store
  useEffect(() => {
    activeTaskCount = tasks.filter(t => t.status === 'downloading' || t.status === 'resuming').length;

    if (activeTaskCount > 0 && !pollingInterval) {
      // Start polling immediately when there are active tasks
      void fetchMetrics();
      pollingInterval = window.setInterval(() => {
        if (activeTaskCount > 0) {
          void fetchMetrics();
        }
      }, 2000);
    } else if (activeTaskCount === 0 && pollingInterval) {
      // Stop polling and reset metrics when no tasks are active
      window.clearInterval(pollingInterval);
      pollingInterval = null;
      metricsState = { dlSpeed: 0, upSpeed: 0, activeConnections: 0 };
      notifyListeners();
    }
  }, [tasks]);

  // Subscribe to metrics changes
  useEffect(() => {
    listeners.add(setState);
    return () => {
      listeners.delete(setState);
    };
  }, []);

  return state;
}
