import { useState, useEffect } from "react";
import { useApp } from "../store";
import { getStorageAppVolumeStats } from "../api/daemon-client";

export const useStorageStats = () => {
  const [availableBytes, setAvailableBytes] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    const fetchStorage = async () => {
      try {
        const stats = await getStorageAppVolumeStats();
        if (!cancelled) {
          setAvailableBytes(stats.available_bytes ?? null);
        }
      } catch {
        if (!cancelled) {
          setAvailableBytes(null);
        }
      }
    };

    void fetchStorage();
    const interval = setInterval(fetchStorage, 10000); // Check every 10 seconds

    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, []);

  return { availableBytes };
};

function formatStorage(bytes: number | null): string {
  if (bytes === null) {
    return "...";
  }
  const gb = bytes / 1024 ** 3;
  if (gb < 1024) {
    return `${Math.floor(gb)} GB`;
  }
  return `${(gb / 1024).toFixed(1)} TB`;
}

export function StorageStats() {
  const { t } = useApp();
  const { availableBytes } = useStorageStats();

  return (
    <div className="flex items-center">
      <span className="text-sm font-black">
        {formatStorage(availableBytes)} {t.storageFree}
      </span>
    </div>
  );
}
