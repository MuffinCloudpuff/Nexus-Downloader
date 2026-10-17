import { useApp } from "../store";

export function NewTaskButton() {
  const { setNewTaskModalOpen, t } = useApp();

  return (
    <button
      onClick={() => setNewTaskModalOpen(true)}
      className="bg-blue-600 hover:bg-blue-500 text-white px-6 py-2 rounded-full font-black text-sm flex items-center gap-2 shadow-lg shadow-blue-500/20 active:scale-95 transition-transform shrink-0"
    >
      <span className="text-lg">+</span> {t.newDownload}
    </button>
  );
}
