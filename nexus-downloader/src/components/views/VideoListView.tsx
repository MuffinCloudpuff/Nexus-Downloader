import { useApp } from "../../store";
import { compareDownloadTasksForDisplay } from "../../lib/task-sort";
import { TaskCard } from "../TaskCard";
import { DownloadTask } from "../../types";

export function VideoListView() {
  const { tasks, t, settings, searchQuery } = useApp();

  let filteredTasks = tasks.filter(task => task.type === 'video');

  if (searchQuery.trim() !== '') {
    const q = searchQuery.toLowerCase();
    filteredTasks = filteredTasks.filter(t => t.name.toLowerCase().includes(q));
  }

  if (filteredTasks.length === 0) {
    return (
      <div className="py-12 flex flex-col items-center justify-center text-black/40 dark:text-white/40 gap-4 bg-black/5 dark:bg-white/[0.02] border border-black/5 dark:border-white/5 rounded-2xl">
        <span className="text-4xl font-black italic">{t.empty}</span>
      </div>
    );
  }

  if (settings.autoOrganizeVideos) {
    const platformTasks = {
      bilibili: [] as DownloadTask[],
      youtube: [] as DownloadTask[],
      twitter: [] as DownloadTask[],
      douyin: [] as DownloadTask[],
      other: [] as DownloadTask[],
    };

    filteredTasks.forEach(task => {
      if (task.platform === 'bilibili') platformTasks.bilibili.push(task);
      else if (task.platform === 'youtube') platformTasks.youtube.push(task);
      else if (task.platform === 'twitter') platformTasks.twitter.push(task);
      else if (task.platform === 'douyin') platformTasks.douyin.push(task);
      else platformTasks.other.push(task);
    });

    platformTasks.bilibili.sort(compareDownloadTasksForDisplay);
    platformTasks.youtube.sort(compareDownloadTasksForDisplay);
    platformTasks.twitter.sort(compareDownloadTasksForDisplay);
    platformTasks.douyin.sort(compareDownloadTasksForDisplay);
    platformTasks.other.sort(compareDownloadTasksForDisplay);

    return (
      <div className="space-y-8">
        {platformTasks.bilibili.length > 0 && (
          <div className="space-y-3">
            <h3 className="font-bold text-sm text-[#fb7299] uppercase flex items-center gap-2">Bilibili</h3>
            {platformTasks.bilibili.map(t => <TaskCard key={t.id} task={t} />)}
          </div>
        )}
        {platformTasks.youtube.length > 0 && (
          <div className="space-y-3">
            <h3 className="font-bold text-sm text-red-500 uppercase flex items-center gap-2">YouTube</h3>
            {platformTasks.youtube.map(t => <TaskCard key={t.id} task={t} />)}
          </div>
        )}
        {platformTasks.twitter.length > 0 && (
          <div className="space-y-3">
            <h3 className="font-bold text-sm text-black dark:text-white uppercase flex items-center gap-2">Twitter(X)</h3>
            {platformTasks.twitter.map(t => <TaskCard key={t.id} task={t} />)}
          </div>
        )}
        {platformTasks.douyin.length > 0 && (
          <div className="space-y-3">
            <h3 className="font-bold text-sm text-[#fe2c55] flex items-center gap-2">抖音</h3>
            {platformTasks.douyin.map(t => <TaskCard key={t.id} task={t} />)}
          </div>
        )}
        {platformTasks.other.length > 0 && (
          <div className="space-y-3">
            <h3 className="font-bold text-sm text-black/40 dark:text-white/40 uppercase">{t.categoryOther}</h3>
            {platformTasks.other.map(t => <TaskCard key={t.id} task={t} />)}
          </div>
        )}
      </div>
    );
  }

  // Default sorting for normal list view
  const sortedTasks = [...filteredTasks].sort(compareDownloadTasksForDisplay);

  return (
    <div className="space-y-3">
      {sortedTasks.map(task => <TaskCard key={task.id} task={task} />)}
    </div>
  );
}
