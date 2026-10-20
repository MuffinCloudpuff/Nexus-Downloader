import { useEffect, useState } from "react";
import { FileArchive, FileVideo, FileText, File, Music, Terminal, Play, Pause, RotateCcw, X, Image } from "lucide-react";
import { motion } from "motion/react";
import { DownloadTask } from "../types";
import { cn } from "../lib/utils";
import { useApp } from "../store";
import { getWindowsFileIcon, revealPathInDesktop } from "../platform/desktop-shell";
import { DEFAULT_DAEMON_URL } from "../api/daemon-client";

export const YoutubeLogo = ({ className }: { className?: string }) => (
  <svg viewBox="0 0 1024 1024" className={className}>
    <rect width="1024" height="1024" rx="256" fill="#FF0000"/>
    <path fill="#FFFFFF" d="M720.5 504.6l-320-184.2c-6.8-3.9-15.5 0.9-15.5 8.7v368.4c0 7.8 8.7 12.6 15.5 8.7l320-184.2c6.5-3.8 6.5-13.6 0-17.4z"/>
  </svg>
);

export const BilibiliLogo = ({ className }: { className?: string }) => (
  <svg viewBox="0 0 1024 1024" className={className}>
    <rect width="1024" height="1024" rx="256" fill="#fb7299"/>
    <path fill="#ffffff" d="M714.49 268.14a45.69 45.69 0 0 1 0 64.6l-50 50h85A142.3 142.3 0 0 1 891.5 525v206.5A142.3 142.3 0 0 1 749.5 873.8h-475a142.3 142.3 0 0 1-142-142.3V525a142.3 142.3 0 0 1 142.2-142.3h85l-50-50a45.7 45.7 0 1 1 64.6-64.6l96 96h84l96-96a45.7 45.7 0 0 1 64.69 0zM274.5 474a50.8 50.8 0 0 0-51 51v206.5a51 51 0 0 0 51 51h475a50.8 50.8 0 0 0 51-51V525a51.1 51.1 0 0 0-51-51h-475zm121 82.5a43 43 0 1 1-86 0 43 43 0 0 1 86 0zm318.5 0a43 43 0 1 1-86 0 43 43 0 0 1 86 0zm-153.5 95a88 88 0 0 1-125.7 0 32 32 0 1 1 45.3-45.2 24.3 24.3 0 0 0 35.1 0 32 32 0 1 1 45.3 45.3z" />
  </svg>
);

export const XLogo = ({ className }: { className?: string }) => (
  <svg viewBox="0 0 1024 1024" className={className}>
    <rect width="1024" height="1024" rx="256" fill="#000000"/>
    <path fill="#ffffff" d="M787.21 210.04h116.71L649.33 501.11 949 813.96h-234.6l-183.74-240.23-210.22 240.23h-116.8l270.83-309.52L162.79 210.04h240.6l166.45 220.21 190.5-220.21H787.21v.01zm-40.82 556.12h64.66L317.07 263.35H247.9l511.4 502.81v.01z" />
  </svg>
);

export const formatBytes = (bytes: number) => {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + " " + sizes[i];
};

// ── Windows shell icon loader ──────────────────────────────────────────

function getTypeIconUrl(ext: string, platform: string | undefined): string | null {
  if (platform === "youtube" || platform === "bilibili" || platform === "twitter" || platform === "douyin") {
    return null;
  }
  return `${DEFAULT_DAEMON_URL}/api/system-icon?ext=${encodeURIComponent(ext)}&v=3`;
}

function fileExtension(value?: string): string {
  const clean = (value || "").split(/[?#]/)[0] || "";
  const name = clean.split(/[\\/]/).pop() || "";
  const index = name.lastIndexOf(".");
  return index >= 0 ? name.slice(index + 1).toLowerCase() : "";
}

function taskIconExtension(task: DownloadTask): string {
  return fileExtension(task.outputPath) || fileExtension(task.name);
}

// ── Platform config builder ──────────────────────────────────────────

export const getPlatformConfig = (
  task: DownloadTask,
  shellIconUrl: string | null = null,
  onShellIconError?: () => void,
) => {
  if (task.platform === "youtube") return { color: "", bg: "", display: <img src="/youtub.jpg" alt="YouTube" className="w-full h-full object-cover rounded-xl" />, progressColor: "bg-red-500" };
  if (task.platform === "bilibili") return { color: "", bg: "", display: <img src="/bilibili.png" alt="Bilibili" className="w-full h-full object-cover rounded-xl" />, progressColor: "bg-[#fb7299]" };
  if (task.platform === "twitter") return { color: "", bg: "", display: <img src="/X.png" alt="X" className="w-full h-full object-cover rounded-xl dark:invert" />, progressColor: "bg-black dark:bg-white" };
  if (task.platform === "douyin") return { color: "", bg: "", display: <img src="/douyin.png" alt="抖音" className="w-full h-full object-contain rounded-xl" />, progressColor: "bg-[#fe2c55]" };

  const ext = taskIconExtension(task);

  // If we have a real Windows shell icon, use it directly
  if (shellIconUrl) {
    let progressColor = "bg-blue-500";
    if (["doc", "docx"].includes(ext)) progressColor = "bg-[#2b579a]";
    else if (["xls", "xlsx", "csv"].includes(ext)) progressColor = "bg-[#217346]";
    else if (["ppt", "pptx"].includes(ext)) progressColor = "bg-[#b7472a]";
    else if (["zip", "7z", "rar", "gz", "tar"].includes(ext)) progressColor = "bg-yellow-600";
    else if (task.type === "video") progressColor = "bg-blue-500";
    else if (task.type === "audio") progressColor = "bg-purple-600";
    else if (task.type === "program") progressColor = "bg-cyan-600";

    return {
      color: "",
      bg: "bg-transparent",
      display: <img src={shellIconUrl} alt="" className="w-9 h-9" onError={onShellIconError} />,
      progressColor,
    };
  }

  // Fallback: Lucide icons while shell icon is loading
  let icon = <File className="w-6 h-6" />;
  let color = "text-black dark:text-white";
  let bg = "bg-black/10 dark:bg-white/10";
  let progressColor = "bg-blue-500";

  if (["doc", "docx"].includes(ext)) {
    icon = <FileText className="w-6 h-6" />;
    color = "text-[#2b579a]";
    bg = "bg-[#2b579a]/10";
    progressColor = "bg-[#2b579a]";
  } else if (["xls", "xlsx", "csv"].includes(ext)) {
    icon = <FileText className="w-6 h-6" />;
    color = "text-[#217346]";
    bg = "bg-[#217346]/10";
    progressColor = "bg-[#217346]";
  } else if (["ppt", "pptx"].includes(ext)) {
    icon = <FileText className="w-6 h-6" />;
    color = "text-[#b7472a]";
    bg = "bg-[#b7472a]/10";
    progressColor = "bg-[#b7472a]";
  } else if (["pdf"].includes(ext)) {
    icon = <FileText className="w-6 h-6" />;
    color = "text-red-500 dark:text-red-400";
    bg = "bg-red-500/10";
  } else if (["txt", "md"].includes(ext)) {
    icon = <FileText className="w-6 h-6" />;
    color = "text-gray-600 dark:text-gray-400";
    bg = "bg-gray-600/10";
  } else {
    switch (task.type) {
      case "video": icon = <FileVideo className="w-6 h-6" />; break;
      case "image": icon = <Image className="w-6 h-6" />; bg = "bg-emerald-600/10"; color = "text-emerald-600 dark:text-emerald-400"; break;
      case "archive": icon = <FileArchive className="w-6 h-6" />; bg = "bg-yellow-600/10"; color = "text-yellow-600 dark:text-yellow-400"; break;
      case "document": icon = <FileText className="w-6 h-6" />; break;
      case "audio": icon = <Music className="w-6 h-6" />; bg = "bg-purple-600/10"; color = "text-purple-600 dark:text-purple-400"; break;
      case "program": icon = <Terminal className="w-6 h-6" />; bg = "bg-cyan-600/10"; color = "text-cyan-600 dark:text-cyan-400"; break;
    }
  }

  return { color, bg, display: icon, progressColor };
};

// ── TaskCard component ────────────────────────────────────────────────

export function TaskCard({ task }: { task: DownloadTask }) {
  const { t, pauseTaskById, resumeTaskById, deleteTaskById, retryTaskById } = useApp();
  const ext = taskIconExtension(task);
  const shellIconUrl = getTypeIconUrl(ext, task.platform);
  const [nativeIcon, setNativeIcon] = useState<{ key: string; url: string } | null>(null);
  const iconKey = `${shellIconUrl}|${task.outputPath}|${task.status}|${task.artifactState}`;
  const [shellIconFailed, setShellIconFailed] = useState(false);

  useEffect(() => {
    let disposed = false;
    setShellIconFailed(false);
    if (shellIconUrl) {
      void getWindowsFileIcon(task.outputPath, ext).then(url => {
        if (!disposed && url) {
          setNativeIcon({ key: iconKey, url });
          setShellIconFailed(false);
        }
      }).catch(() => { /* Browser preview and older shells retain the HTTP fallback. */ });
    }
    return () => { disposed = true; };
  }, [iconKey, shellIconUrl, task.outputPath, ext]);

  const platform = getPlatformConfig(
    task,
    shellIconFailed ? null : nativeIcon?.key === iconKey ? nativeIcon.url : shellIconUrl,
    () => setShellIconFailed(true),
  );
  const progress = Math.min(100, Math.max(0, task.progress * 100));
  const artifactMissing = task.artifactState === "missing";
  const isActiveTransfer = task.status === "downloading" || task.status === "resuming";
  const canRevealArtifact = Boolean(task.outputPath && task.status === "completed" && !artifactMissing);
  const canRetry = task.status === "completed" || task.status === "error";

  const revealArtifact = () => {
    if (canRevealArtifact) {
      void revealPathInDesktop(task.outputPath);
    }
  };

  const getStatusText = () => {
    switch (task.status) {
      case "downloading": return t.statusDownloading;
      case "resuming": return t.statusResuming;
      case "paused": return t.statusPaused;
      case "completed": return t.statusCompleted;
      case "error": return t.statusFailed;
      case "waiting": return "Waiting";
      default: return String(task.status).toUpperCase();
    }
  };

  const getTypeText = () => {
    switch (task.type) {
      case "video": return t.categoryVideo;
      case "audio": return t.categoryAudio;
      case "image": return t.categoryImage;
      case "document": return t.categoryDocument;
      case "archive": return t.categoryArchive;
      case "program": return t.categoryProgram;
      case "other": return t.categoryOther;
      default: return task.type;
    }
  };

  return (
    <div className={cn(
      "bg-white dark:bg-white/[0.03] border border-black/10 dark:border-white/10 shadow-sm dark:shadow-none p-5 rounded-2xl flex items-center gap-6",
      artifactMissing && "bg-neutral-100 dark:bg-white/[0.02] border-neutral-300 dark:border-white/10 opacity-70 grayscale"
    )}>
      <button
        type="button"
        onClick={revealArtifact}
        disabled={!canRevealArtifact}
        className={cn(
          "w-12 h-12 rounded-xl flex items-center justify-center font-bold text-lg shrink-0",
          platform.bg,
          platform.color,
          artifactMissing && "bg-neutral-200 dark:bg-white/10 text-neutral-500",
          canRevealArtifact && "cursor-pointer hover:ring-2 hover:ring-black/10 dark:hover:ring-white/20",
          !canRevealArtifact && "cursor-default",
        )}
        title={artifactMissing ? "File missing" : task.outputPath}
      >
        {platform.display}
      </button>

      <div className="flex-grow min-w-0">
        <div className="flex justify-between items-start mb-2">
          <div className="truncate pr-4">
            <button
              type="button"
              onClick={revealArtifact}
              disabled={!canRevealArtifact}
              className={cn(
                "block max-w-full text-left font-bold text-sm leading-none truncate",
                canRevealArtifact && "cursor-pointer hover:underline",
                !canRevealArtifact && "cursor-default",
              )}
              title={artifactMissing ? "File missing" : task.outputPath || task.name}
            >
              {task.name}
            </button>
            <p className="text-[10px] text-black/40 dark:text-white/40 mt-1 font-mono uppercase">
              {artifactMissing ? t.statusMissing : getTypeText()} • {formatBytes(task.downloaded)} / {formatBytes(task.size)}
            </p>
          </div>
          <span className="font-black text-xl italic">{progress.toFixed(0)}%</span>
        </div>
        <div className="h-1.5 bg-black/10 dark:bg-white/10 rounded-full overflow-hidden">
          <motion.div
            className={cn(
              "h-full",
              artifactMissing ? "bg-neutral-400 dark:bg-white/20" : platform.progressColor,
              isActiveTransfer && !artifactMissing ? "shadow-[0_0_15px_rgba(59,130,246,0.5)]" : "",
            )}
            initial={{ width: 0 }}
            animate={{ width: `${progress}%` }}
          />
        </div>
      </div>

      <div className="flex flex-col items-end gap-1 shrink-0">
        <span className="text-xs font-bold font-mono uppercase">
          {artifactMissing ? t.statusMissing : task.status === "downloading" ? `${formatBytes(task.speed)}/s` : getStatusText()}
        </span>
        <div className="flex gap-2">
          {isActiveTransfer ? (
             <button
               onClick={() => void pauseTaskById(task.id)}
               className="p-2 hover:bg-black/10 dark:hover:bg-white/10 rounded-lg text-black/40 dark:text-white/40 transition-colors"
               title={t.statusPaused}
             >
               <Pause className="w-4 h-4" />
             </button>
          ) : task.status === "paused" ? (
             <button
               onClick={() => void resumeTaskById(task.id)}
               className="p-2 hover:bg-black/10 dark:hover:bg-white/10 rounded-lg text-black/40 dark:text-white/40 transition-colors"
               title={t.statusDownloading}
             >
               <Play className="w-4 h-4 ml-0.5" />
             </button>
          ) : canRetry ? (
             <button
               onClick={() => void retryTaskById(task.id)}
               className="p-2 hover:bg-black/10 dark:hover:bg-white/10 rounded-lg text-black/40 dark:text-white/40 transition-colors"
               title={t.downloadBtn}
             >
               <RotateCcw className="w-4 h-4" />
             </button>
          ) : null}
          <button
            onClick={() => void deleteTaskById(task.id)}
            className="p-2 hover:bg-black/10 dark:hover:bg-white/10 rounded-lg text-black/40 dark:text-white/40 hover:text-red-500 dark:hover:text-red-400 transition-colors"
            title="Delete"
          >
            <X className="w-4 h-4" />
          </button>
        </div>
      </div>
    </div>
  );
}
