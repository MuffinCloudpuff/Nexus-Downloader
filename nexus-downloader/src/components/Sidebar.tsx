import { motion } from "motion/react";
import { Download, CheckCircle2, Clock, Trash2, Settings, HardDriveDownload, UserCircle, Tv, Film, Music, FileText, FileArchive, Terminal, File, Image } from "lucide-react";
import { useApp } from "../store";
import { cn } from "../lib/utils";
import { GlobalSpeed } from "./GlobalSpeed";
import { Logo } from "./Logo";

export function Sidebar() {
  const { activeFilter, setActiveFilter, setBilibiliModalOpen, setSettingsModalOpen, t } = useApp();

  const navItems = [
    { id: 'all', icon: HardDriveDownload, label: t.categoryAll },
    { id: 'video', icon: Film, label: t.categoryVideo },
    { id: 'audio', icon: Music, label: t.categoryAudio },
    { id: 'image', icon: Image, label: t.categoryImage },
    { id: 'document', icon: FileText, label: t.categoryDocument },
    { id: 'archive', icon: FileArchive, label: t.categoryArchive },
    { id: 'program', icon: Terminal, label: t.categoryProgram },
    { id: 'other', icon: File, label: t.categoryOther },
  ] as const;

  return (
    <aside className="w-64 shrink-0 border-r border-black/10 dark:border-white/10 flex flex-col p-6 z-20 min-h-[680px] overflow-hidden">
      <div className="mb-12 shrink-0">
        <Logo className="mb-2" />
      </div>

      <nav className="shrink-0">
        <ul className="space-y-2">
          {navItems.map((item) => (
            <li key={item.id}>
              <button
                onClick={() => setActiveFilter(item.id)}
                className={cn(
                  "w-full flex items-center gap-3 py-2 relative group",
                  activeFilter === item.id
                    ? "text-blue-500 font-black text-2xl italic tracking-tight"
                    : "text-black/50 hover:text-black dark:text-white/50 dark:hover:text-white font-bold text-xl tracking-tight transition-colors"
                )}
              >
                {activeFilter === item.id && (
                  <span className="w-2 h-2 rounded-full bg-blue-500 shrink-0" />
                )}
                {activeFilter !== item.id && (
                  <item.icon className="w-5 h-5 opacity-70 shrink-0" />
                )}
                <span className="whitespace-nowrap pr-2">{item.label}</span>
              </button>
            </li>
          ))}
        </ul>
      </nav>

      <div className="min-h-2 flex-[1_1_3rem]" />

      <button
        onClick={() => setBilibiliModalOpen(true)}
        className="w-full flex items-center gap-3 py-2 text-black/50 hover:text-black dark:text-white/50 dark:hover:text-white transition-all duration-300 group shrink-0"
      >
        <img src="/bilibili.png" alt="Bilibili" className="w-5 h-5 shrink-0 rounded object-cover" />
        <span className="font-bold text-xl tracking-tight whitespace-nowrap">{t.bilibiliAuth}</span>
        <span className="text-[10px] font-bold text-red-500 uppercase px-1.5 py-0.5 rounded-full bg-red-500/10 ml-auto whitespace-nowrap">!</span>
      </button>

      <div className="min-h-2 flex-[1_1_3rem]" />

      <div className="shrink-0 flex flex-col gap-4">
        <button
          onClick={() => setSettingsModalOpen(true)}
          className="w-full flex items-center gap-3 py-2 text-black/50 hover:text-black dark:text-white/50 dark:hover:text-white transition-all duration-300 group"
        >
          <Settings className="w-5 h-5 opacity-70 shrink-0" />
          <span className="font-bold text-xl tracking-tight whitespace-nowrap pr-2">{t.settings}</span>
        </button>
        <GlobalSpeed />
      </div>
    </aside>
  );
}
