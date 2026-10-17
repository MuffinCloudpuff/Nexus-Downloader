import { motion } from "motion/react";
import { useApp } from "../store";
import { cn } from "../lib/utils";
import { AllListView } from "./views/AllListView";
import { VideoListView } from "./views/VideoListView";
import { GenericTypeListView } from "./views/GenericTypeListView";

export function DownloadList() {
  const { tasks, activeFilter, t, settings, updateSettings, searchQuery } = useApp();

  const titleMap = {
    all: t.categoryAll,
    video: t.categoryVideo,
    audio: t.categoryAudio,
    image: t.categoryImage,
    document: t.categoryDocument,
    archive: t.categoryArchive,
    program: t.categoryProgram,
    other: t.categoryOther,
  };

  let activeTasks =
    activeFilter === "all" ? tasks : tasks.filter((task) => task.type === activeFilter);

  if (searchQuery.trim() !== "") {
    const query = searchQuery.toLowerCase();
    activeTasks = activeTasks.filter((task) => task.name.toLowerCase().includes(query));
  }

  return (
    <div className="flex-1 overflow-y-auto p-8 flex flex-col gap-6">
      <section className="flex-grow">
        <div className="flex justify-between items-end mb-4">
          <div className="flex items-center gap-6">
            <div className="overflow-hidden py-1">
              <motion.h2
                key={activeFilter}
                initial={{ x: "-100%", skewX: 20 }}
                animate={{ x: 0, skewX: 0 }}
                transition={{ type: "spring", stiffness: 400, damping: 25 }}
                className="text-4xl font-black italic tracking-tighter uppercase whitespace-nowrap"
              >
                {titleMap[activeFilter]}<span className="text-blue-500">_</span>
              </motion.h2>
            </div>
            {activeFilter === "video" && (
              <button
                onClick={() => updateSettings({ autoOrganizeVideos: !settings.autoOrganizeVideos })}
                className={cn(
                  "flex items-center gap-2 px-3 py-1.5 rounded-full border text-[10px] uppercase tracking-widest font-bold transition-all mb-1",
                  settings.autoOrganizeVideos
                    ? "bg-blue-500/10 border-blue-500/30 text-blue-600 dark:text-blue-400"
                    : "bg-black/5 dark:bg-white/5 border-black/10 dark:border-white/10 text-black/40 dark:text-white/40 hover:text-black dark:hover:text-white hover:bg-black/10 dark:hover:bg-white/10",
                )}
              >
                <div
                  className={cn(
                    "w-2 h-2 rounded-full transition-all",
                    settings.autoOrganizeVideos
                      ? "bg-blue-500 shadow-[0_0_8px_rgba(59,130,246,0.5)]"
                      : "bg-black/20 dark:bg-white/20",
                  )}
                />
                {t.autoOrganize}
              </button>
            )}
          </div>
          <span className="text-xs font-bold text-black/40 dark:text-white/40 mb-1">
            {activeTasks.length} {t.tasksCount}
          </span>
        </div>

        {activeFilter === "all" && <AllListView />}
        {activeFilter === "video" && <VideoListView />}
        {activeFilter !== "all" && activeFilter !== "video" && (
          <GenericTypeListView type={activeFilter} />
        )}
      </section>
    </div>
  );
}
