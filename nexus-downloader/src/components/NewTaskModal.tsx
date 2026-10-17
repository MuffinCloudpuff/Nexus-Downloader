import { useState } from "react";
import { X, Link as LinkIcon, Download } from "lucide-react";
import { useApp } from "../store";
import { ModalOverlay } from "./ModalOverlay";

export function NewTaskModal() {
  const { isNewTaskModalOpen, setNewTaskModalOpen, createDownloadFromUrl, t } = useApp();
  const [url, setUrl] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submitDownload() {
    const trimmedUrl = url.trim();
    if (!trimmedUrl || submitting) {
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      await createDownloadFromUrl(trimmedUrl);
      setUrl("");
      setNewTaskModalOpen(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create download");
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <ModalOverlay isOpen={isNewTaskModalOpen} onClose={() => setNewTaskModalOpen(false)}>
      <div className="p-8">
        <div className="flex items-center justify-between mb-8">
          <h2 className="text-3xl font-black italic tracking-tighter">{t.newTaskTitle}</h2>
          <button onClick={() => setNewTaskModalOpen(false)} className="p-2 rounded-full hover:bg-black/10 dark:hover:bg-white/10 transition-colors text-black/40 dark:text-white/40 hover:text-black dark:hover:text-white">
            <X className="w-6 h-6" />
          </button>
        </div>

        <div className="space-y-6">
          <div>
            <label className="block text-[10px] font-bold text-black/40 dark:text-white/40 uppercase tracking-widest mb-2">{t.resourceUrl}</label>
            <div className="relative group">
              <LinkIcon className="absolute left-4 top-1/2 -translate-y-1/2 w-5 h-5 text-black/40 dark:text-white/40 group-focus-within:text-blue-500 transition-colors" />
              <input
                type="text"
                placeholder={t.pasteUrl}
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    void submitDownload();
                  }
                }}
                className="w-full bg-black/5 dark:bg-white/5 border border-black/10 dark:border-white/10 rounded-xl py-4 pl-12 pr-4 text-black dark:text-white text-sm font-bold placeholder:text-black/30 dark:placeholder:text-white/30 focus:outline-none focus:border-blue-500/50 transition-all bg-transparent"
                autoFocus
              />
            </div>
            {error && (
              <p className="mt-2 text-xs font-bold text-red-500 dark:text-red-400">{error}</p>
            )}
          </div>

          <div className="pt-4">
             <button
               onClick={() => void submitDownload()}
               disabled={submitting || url.trim() === ""}
               className="w-full py-4 rounded-xl bg-blue-600 hover:bg-blue-500 disabled:bg-blue-600/50 disabled:cursor-not-allowed text-white font-black uppercase text-sm tracking-widest transition-colors flex items-center justify-center gap-2 shadow-lg shadow-blue-900/20"
             >
              <Download className="w-5 h-5" />
              <span>{submitting ? "..." : t.downloadBtn}</span>
            </button>
          </div>
        </div>
      </div>
    </ModalOverlay>
  );
}
