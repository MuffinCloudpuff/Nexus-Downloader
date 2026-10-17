import { useApp } from "../../store";
import { compareDownloadTasksForDisplay } from "../../lib/task-sort";
import { TaskCard } from "../TaskCard";
import { TaskType } from "../../types";

export function GenericTypeListView({ type }: { type: TaskType }) {
  const { tasks, t, searchQuery } = useApp();

  let filteredTasks = tasks.filter(task => task.type === type);

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

  const sortedTasks = [...filteredTasks].sort(compareDownloadTasksForDisplay);

  return (
    <div className="space-y-3">
      {sortedTasks.map(task => <TaskCard key={task.id} task={task} />)}
    </div>
  );
}
