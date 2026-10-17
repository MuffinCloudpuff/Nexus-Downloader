import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type ReactNode,
  type SetStateAction,
} from "react";
import {
  cancelTask,
  createTask,
  deleteTask,
  getSettings,
  getTasks,
  patchSettings,
  pauseTask,
  retryTask,
  resumeTask,
} from "../api/daemon-client";
import { applyTaskEvent, taskSummaryToDownloadTask } from "../adapters/task-adapter";
import { translations, type Language } from "../i18n";
import { getLaunchOnStartup, syncLaunchOnStartup } from "../platform/desktop-shell";
import type { AppSettingsDto, CategoryDownloadPaths, DownloadDirectoryMode, TaskEvent, TaskSummary } from "../protocol/daemon";
import type { DownloadTask, TaskType } from "../types";

export interface DownloadPaths {
  video: string;
  audio: string;
  image: string;
  document: string;
  archive: string;
  program: string;
  other: string;
}

export interface Settings {
  threads: number | "";
  language: Language;
  theme: "light" | "dark";
  autoOrganizeVideos: boolean;
  closeBehavior: "exit" | "minimize";
  launchOnStartup: boolean;
  directoryMode: DownloadDirectoryMode;
  flatPath: string;
  categorizedRoot: string;
  customPaths: DownloadPaths;
  systemPaths: DownloadPaths;
}

export type ConnectionState = "connecting" | "connected" | "disconnected" | "unauthorized";

export interface ConnectionStatus {
  state: ConnectionState;
  message: string;
}

export type SettingsPatch = Omit<Partial<Settings>, "customPaths" | "systemPaths"> & {
  customPaths?: Partial<DownloadPaths>;
};

interface AppState {
  tasks: DownloadTask[];
  settings: Settings;
  connection: ConnectionStatus;
  isBilibiliModalOpen: boolean;
  isNewTaskModalOpen: boolean;
  isSettingsModalOpen: boolean;
  activeFilter: "all" | TaskType;
  searchQuery: string;
  appLoaded: boolean;
  t: typeof translations.en;
  setTasks: Dispatch<SetStateAction<DownloadTask[]>>;
  setConnection: Dispatch<SetStateAction<ConnectionStatus>>;
  updateSettings: (newSettings: SettingsPatch) => void;
  replaceTaskSnapshot: (snapshot: TaskSummary[]) => void;
  applyDaemonTaskEvent: (event: TaskEvent) => void;
  createDownloadFromUrl: (url: string) => Promise<void>;
  pauseTaskById: (taskId: string) => Promise<void>;
  resumeTaskById: (taskId: string) => Promise<void>;
  cancelTaskById: (taskId: string) => Promise<void>;
  deleteTaskById: (taskId: string) => Promise<void>;
  retryTaskById: (taskId: string) => Promise<void>;
  setBilibiliModalOpen: (open: boolean) => void;
  setNewTaskModalOpen: (open: boolean) => void;
  setSettingsModalOpen: (open: boolean) => void;
  setActiveFilter: (filter: "all" | TaskType) => void;
  setSearchQuery: (query: string) => void;
  setAppLoaded: (loaded: boolean) => void;
}

const AppContext = createContext<AppState | undefined>(undefined);

const DEFAULT_PATHS: DownloadPaths = {
  video: "D:/Nexus/Video",
  audio: "D:/Nexus/Audio",
  image: "D:/Nexus/Image",
  document: "D:/Nexus/Document",
  archive: "D:/Nexus/Archive",
  program: "D:/Nexus/Program",
  other: "D:/Nexus/Other",
};

const MIN_CHUNK_CONNECTIONS = 8;
const MAX_CHUNK_CONNECTIONS = 256;
const DEFAULT_CHUNK_CONNECTIONS = MAX_CHUNK_CONNECTIONS;

const DEFAULT_SETTINGS: Settings = {
  threads: DEFAULT_CHUNK_CONNECTIONS,
  language: "zh",
  theme: "light",
  autoOrganizeVideos: false,
  closeBehavior: "minimize",
  launchOnStartup: false,
  directoryMode: "categorized",
  flatPath: "D:/Nexus/Downloads",
  categorizedRoot: "D:/Nexus",
  customPaths: DEFAULT_PATHS,
  systemPaths: DEFAULT_PATHS,
};

const LOCAL_SETTINGS_KEY = "nexus-downloader.uiSettings";

function loadLocalSettings(): Settings {
  try {
    const raw = window.localStorage.getItem(LOCAL_SETTINGS_KEY);
    if (!raw) {
      return DEFAULT_SETTINGS;
    }
    const parsed = JSON.parse(raw) as Partial<Settings>;
    return {
      ...DEFAULT_SETTINGS,
      ...parsed,
      customPaths: {
        ...DEFAULT_PATHS,
        ...(parsed.customPaths ?? {}),
      },
      systemPaths: {
        ...DEFAULT_PATHS,
        ...(parsed.systemPaths ?? {}),
      },
      threads: normalizeThreads(parsed.threads),
    };
  } catch {
    return DEFAULT_SETTINGS;
  }
}

function normalizeThreads(value: Settings["threads"] | number | string | undefined): Settings["threads"] {
  if (value === "") {
    return "";
  }
  const parsed = Number(value ?? DEFAULT_SETTINGS.threads);
  if (!Number.isFinite(parsed)) {
    return DEFAULT_SETTINGS.threads;
  }
  return Math.min(MAX_CHUNK_CONNECTIONS, Math.max(MIN_CHUNK_CONNECTIONS, Math.round(parsed)));
}

function settingsFromDaemon(current: Settings, daemon: AppSettingsDto): Settings {
  return {
    ...current,
    threads: normalizeThreads(daemon.max_chunk_connections),
    directoryMode: daemon.directory_mode || "categorized",
    flatPath: daemon.download_folder || "D:/Nexus/Downloads",
    categorizedRoot: daemon.categorized_root || "D:/Nexus",
    customPaths: {
      ...current.customPaths,
      ...daemon.custom_paths,
    },
    systemPaths: {
      ...current.systemPaths,
      ...daemon.system_paths,
    },
  };
}

function daemonPatchFromSettings(settings: SettingsPatch) {
  const patch: {
    max_chunk_connections?: number;
    directory_mode?: DownloadDirectoryMode;
    download_folder?: string;
    categorized_root?: string;
    custom_paths?: Partial<CategoryDownloadPaths>;
  } = {};

  if (typeof settings.threads === "number") {
    const threads = normalizeThreads(settings.threads);
    patch.max_chunk_connections = typeof threads === "number" ? threads : DEFAULT_CHUNK_CONNECTIONS;
  }
  if (settings.directoryMode) {
    patch.directory_mode = settings.directoryMode;
  }
  if (settings.flatPath) {
    patch.download_folder = settings.flatPath;
  }
  if (settings.categorizedRoot) {
    patch.categorized_root = settings.categorizedRoot;
  }
  if (settings.customPaths) {
    patch.custom_paths = settings.customPaths;
  }

  return patch;
}

export function AppProvider({ children }: { children: ReactNode }) {
  const [tasks, setTasks] = useState<DownloadTask[]>([]);
  const [settings, setSettings] = useState<Settings>(loadLocalSettings);
  const [connection, setConnection] = useState<ConnectionStatus>({
    state: "connecting",
    message: "Connecting to daemon",
  });
  const [isBilibiliModalOpen, setBilibiliModalOpen] = useState(false);
  const [isNewTaskModalOpen, setNewTaskModalOpen] = useState(false);
  const [isSettingsModalOpen, setSettingsModalOpen] = useState(false);
  const [activeFilter, setActiveFilter] = useState<"all" | TaskType>("all");
  const [searchQuery, setSearchQuery] = useState("");
  const [appLoaded, setAppLoaded] = useState(false);
  const settingsSaveSeqRef = useRef(0);

  useEffect(() => {
    window.localStorage.setItem(LOCAL_SETTINGS_KEY, JSON.stringify(settings));
  }, [settings]);

  useEffect(() => {
    let cancelled = false;

    async function loadDaemonSettings() {
      try {
        const daemonSettings = await getSettings();
        if (!cancelled) {
          setSettings((current) => settingsFromDaemon(current, daemonSettings));
        }
      } catch {
        // The task sync effect owns user-visible daemon connection state.
      }
    }

    void loadDaemonSettings();
    void getLaunchOnStartup().then((enabled) => {
      if (!cancelled && enabled !== null) {
        setSettings((current) => ({ ...current, launchOnStartup: enabled }));
      }
    }).catch(() => {
      // Keep the local setting when platform autostart state cannot be read.
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const replaceTaskSnapshot = useCallback((snapshot: TaskSummary[]) => {
    setTasks((current) => {
      const currentById = new Map(current.map((task) => [task.id, task]));
      return snapshot.map((task) => taskSummaryToDownloadTask(task, currentById.get(task.task_id)));
    });
  }, []);

  const applyDaemonTaskEvent = useCallback((event: TaskEvent) => {
    setTasks((current) => {
      if (event.type === "task_deleted") {
        return current.filter((task) => task.id !== event.task_id);
      }

      const exists = current.some((task) => task.id === event.task_id);
      if (!exists) {
        const placeholder: DownloadTask = {
          id: event.task_id,
          name: event.task_id,
          url: "",
          size: 0,
          downloaded: 0,
          progress: 0,
          status: "waiting",
          speed: 0,
          eta: 0,
          type: "other",
          createdAt: Date.now(),
          artifactState: "unknown",
        };
        return [...current, applyTaskEvent(placeholder, event)];
      }
      return current.map((task) => (task.id === event.task_id ? applyTaskEvent(task, event) : task));
    });
  }, []);

  const updateSettings = useCallback((newSettings: SettingsPatch) => {
    const normalized: SettingsPatch = { ...newSettings };
    if (newSettings.threads !== undefined) {
      normalized.threads = normalizeThreads(newSettings.threads);
    }

    setSettings((current) => ({
      ...current,
      ...normalized,
      customPaths: newSettings.customPaths ? { ...current.customPaths, ...newSettings.customPaths } : current.customPaths,
    }));

    if (typeof normalized.launchOnStartup === "boolean") {
      void syncLaunchOnStartup(normalized.launchOnStartup).catch((error) => {
        const message = error instanceof Error ? error.message : "Startup setting save failed";
        setConnection({ state: "disconnected", message });
        void getLaunchOnStartup().then((enabled) => {
          if (enabled !== null) {
            setSettings((current) => ({ ...current, launchOnStartup: enabled }));
          }
        });
      });
    }

    const daemonPatch = daemonPatchFromSettings(normalized);
    if (Object.keys(daemonPatch).length > 0) {
      const saveSeq = ++settingsSaveSeqRef.current;
      void patchSettings(daemonPatch)
        .then((daemonSettings) => {
          if (saveSeq === settingsSaveSeqRef.current) {
            setSettings((current) => settingsFromDaemon(current, daemonSettings));
            setConnection({ state: "connected", message: "Daemon connected" });
          }
        })
        .catch((error) => {
          const message = error instanceof Error ? error.message : "Settings save failed";
          setConnection({ state: "disconnected", message });
          void getSettings()
            .then((daemonSettings) => {
              if (saveSeq === settingsSaveSeqRef.current) {
                setSettings((current) => settingsFromDaemon(current, daemonSettings));
              }
            })
            .catch(() => {
              // Keep the optimistic local value visible when the daemon is unreachable.
            });
        });
    }
  }, [setConnection]);

  const createDownloadFromUrl = useCallback(async (url: string) => {
    const requestId = crypto.randomUUID();
    await createTask({
      request_id: requestId,
      source: "desktop_manual",
      title: url,
      payload: { url },
      client_context: {
        protocol_version: 1,
        trace_id: `trace_desktop_${crypto.randomUUID()}`,
      },
    });
    replaceTaskSnapshot(await getTasks());
  }, [replaceTaskSnapshot]);

  const pauseTaskById = useCallback(async (taskId: string) => {
    await pauseTask(taskId);
  }, []);

  const resumeTaskById = useCallback(async (taskId: string) => {
    await resumeTask(taskId);
  }, []);

  const cancelTaskById = useCallback(async (taskId: string) => {
    await cancelTask(taskId);
  }, []);

  const deleteTaskById = useCallback(async (taskId: string) => {
    await deleteTask(taskId);
    replaceTaskSnapshot(await getTasks());
  }, [replaceTaskSnapshot]);

  const retryTaskById = useCallback(async (taskId: string) => {
    await retryTask(taskId);
    replaceTaskSnapshot(await getTasks());
  }, [replaceTaskSnapshot]);

  const t = translations[settings.language];

  const value = useMemo<AppState>(
    () => ({
      tasks,
      settings,
      connection,
      isBilibiliModalOpen,
      isNewTaskModalOpen,
      isSettingsModalOpen,
      activeFilter,
      searchQuery,
      appLoaded,
      t,
      setTasks,
      setConnection,
      updateSettings,
      replaceTaskSnapshot,
      applyDaemonTaskEvent,
      createDownloadFromUrl,
      pauseTaskById,
      resumeTaskById,
      cancelTaskById,
      deleteTaskById,
      retryTaskById,
      setBilibiliModalOpen,
      setNewTaskModalOpen,
      setSettingsModalOpen,
      setActiveFilter,
      setSearchQuery,
      setAppLoaded,
    }),
    [
      tasks,
      settings,
      connection,
      isBilibiliModalOpen,
      isNewTaskModalOpen,
      isSettingsModalOpen,
      activeFilter,
      searchQuery,
      appLoaded,
      t,
      updateSettings,
      replaceTaskSnapshot,
      applyDaemonTaskEvent,
      createDownloadFromUrl,
      pauseTaskById,
      resumeTaskById,
      cancelTaskById,
      deleteTaskById,
      retryTaskById,
    ],
  );

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp() {
  const context = useContext(AppContext);
  if (!context) {
    throw new Error("useApp must be used within an AppProvider");
  }
  return context;
}
