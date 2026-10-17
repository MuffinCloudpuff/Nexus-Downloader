import { X, FolderOpen } from "lucide-react";
import { useApp } from "../store";
import { ModalOverlay } from "./ModalOverlay";
import { cn } from "../lib/utils";
import { directoryFromFileInput, pickDirectory } from "../platform/desktop-file-dialog";

export function SettingsModal() {
  const { isSettingsModalOpen, setSettingsModalOpen, settings, updateSettings, t } = useApp();

  return (
    <ModalOverlay isOpen={isSettingsModalOpen} onClose={() => setSettingsModalOpen(false)} className="w-full max-w-md rounded-3xl shadow-2xl overflow-hidden relative font-sans bg-blue-600 border-none">
      <div className="bg-blue-600 p-8 text-white relative max-h-[90vh] overflow-y-auto">
        <div className="flex items-center justify-between mb-8">
          <h2 className="text-3xl font-black italic tracking-tighter uppercase">{t.engineSettings}</h2>
          <button onClick={() => setSettingsModalOpen(false)} className="p-2 rounded-full hover:bg-white/10 transition-colors text-white/80 hover:text-white shrink-0">
            <X className="w-6 h-6" />
          </button>
        </div>

        <div className="space-y-8">
          <div>
            <label className="block text-[10px] uppercase font-bold text-white/80 tracking-widest mb-3">
              {t.language}
            </label>
            <div className="flex gap-2">
              <button
                onClick={() => updateSettings({ language: 'zh' })}
                className={cn(
                  "flex-1 py-3 rounded-lg text-xs font-black tracking-widest transition-colors",
                  settings.language === 'zh' ? "bg-white text-blue-600 shadow-lg" : "bg-black/20 text-white/50 hover:bg-black/40 hover:text-white"
                )}
              >
                中文
              </button>
              <button
                onClick={() => updateSettings({ language: 'en' })}
                className={cn(
                  "flex-1 py-3 rounded-lg text-xs font-black uppercase tracking-widest transition-colors",
                  settings.language === 'en' ? "bg-white text-blue-600 shadow-lg" : "bg-black/20 text-white/50 hover:bg-black/40 hover:text-white"
                )}
              >
                English
              </button>
            </div>
          </div>

          <div>
            <label className="block text-[10px] uppercase font-bold text-white/80 tracking-widest mb-3">
              {t.theme}
            </label>
            <div className="flex gap-2">
              <button
                onClick={() => updateSettings({ theme: 'light' })}
                className={cn(
                  "flex-1 py-3 rounded-lg text-xs font-black uppercase tracking-widest transition-colors",
                  settings.theme === 'light' ? "bg-white text-blue-600 shadow-lg" : "bg-black/20 text-white/50 hover:bg-black/40 hover:text-white"
                )}
              >
                {t.light}
              </button>
              <button
                onClick={() => updateSettings({ theme: 'dark' })}
                className={cn(
                  "flex-1 py-3 rounded-lg text-xs font-black uppercase tracking-widest transition-colors",
                  settings.theme === 'dark' ? "bg-white text-blue-600 shadow-lg" : "bg-black/20 text-white/50 hover:bg-black/40 hover:text-white"
                )}
              >
                {t.dark}
              </button>
            </div>
          </div>

          <div>
            <label className="block text-[10px] uppercase font-bold text-white/80 tracking-widest mb-3" suppressHydrationWarning>
              {t.closeBehavior}
            </label>
            <div className="flex gap-2">
              <button
                onClick={() => updateSettings({ closeBehavior: 'minimize' })}
                className={cn(
                  "flex-1 py-3 rounded-lg text-xs font-black uppercase tracking-widest transition-colors",
                  settings.closeBehavior === 'minimize' ? "bg-white text-blue-600 shadow-lg" : "bg-black/20 text-white/50 hover:bg-black/40 hover:text-white"
                )}
                suppressHydrationWarning
              >
                {t.closeMinimize}
              </button>
              <button
                onClick={() => updateSettings({ closeBehavior: 'exit' })}
                className={cn(
                  "flex-1 py-3 rounded-lg text-xs font-black uppercase tracking-widest transition-colors",
                  settings.closeBehavior === 'exit' ? "bg-white text-blue-600 shadow-lg" : "bg-black/20 text-white/50 hover:bg-black/40 hover:text-white"
                )}
                suppressHydrationWarning
              >
                {t.closeExit}
              </button>
            </div>
          </div>

          <div>
            <label className="block text-[10px] uppercase font-bold text-white/80 tracking-widest mb-3" suppressHydrationWarning>
              {t.launchOnStartup}
            </label>
            <button
              type="button"
              onClick={() => updateSettings({ launchOnStartup: !settings.launchOnStartup })}
              className={cn(
                "w-full py-3 rounded-lg text-xs font-black uppercase tracking-widest transition-colors",
                settings.launchOnStartup ? "bg-white text-blue-600 shadow-lg" : "bg-black/20 text-white/50 hover:bg-black/40 hover:text-white"
              )}
              suppressHydrationWarning
            >
              {settings.launchOnStartup ? t.launchOnStartupOn : t.launchOnStartupOff}
            </button>
          </div>

          <div className="pt-6 border-t border-white/10 mt-6">
            <label className="block text-[10px] uppercase font-bold text-white/80 tracking-widest mb-4">
              {t.dirMode}
            </label>
            <div className="flex flex-col gap-2 mb-6">
              <button
                type="button"
                onClick={() => updateSettings({ directoryMode: 'flat' })}
                className={cn(
                  "w-full py-2.5 rounded-lg text-xs font-black tracking-widest transition-colors",
                  settings.directoryMode === 'flat' ? "bg-white text-blue-600 shadow-lg" : "bg-black/20 text-white/50 hover:bg-black/40 hover:text-white"
                )}
                suppressHydrationWarning
              >
                {t.dirModeFlat}
              </button>
              <button
                type="button"
                onClick={() => updateSettings({ directoryMode: 'categorized' })}
                className={cn(
                  "w-full py-2.5 rounded-lg text-xs font-black tracking-widest transition-colors",
                  settings.directoryMode === 'categorized' ? "bg-white text-blue-600 shadow-lg" : "bg-black/20 text-white/50 hover:bg-black/40 hover:text-white"
                )}
                suppressHydrationWarning
              >
                {t.dirModeCategorized}
              </button>
              <button
                type="button"
                onClick={() => updateSettings({ directoryMode: 'system_default' })}
                className={cn(
                  "w-full py-2.5 rounded-lg text-xs font-black tracking-widest transition-colors",
                  settings.directoryMode === 'system_default' ? "bg-white text-blue-600 shadow-lg" : "bg-black/20 text-white/50 hover:bg-black/40 hover:text-white"
                )}
                suppressHydrationWarning
              >
                {t.dirModeSystemDefault}
              </button>
              <button
                type="button"
                onClick={() => updateSettings({ directoryMode: 'custom' })}
                className={cn(
                  "w-full py-2.5 rounded-lg text-xs font-black tracking-widest transition-colors",
                  settings.directoryMode === 'custom' ? "bg-white text-blue-600 shadow-lg" : "bg-black/20 text-white/50 hover:bg-black/40 hover:text-white"
                )}
                suppressHydrationWarning
              >
                {t.dirModeCustom}
              </button>
            </div>

            {settings.directoryMode === 'flat' && (
              <div className="flex flex-col gap-4">
                <label className="block text-[10px] uppercase font-bold text-white/80 tracking-widest">
                  {t.downloadFolder}
                </label>
                <div className="relative">
                  <input
                    type="text"
                    readOnly
                    value={settings.flatPath}
                    onClick={async () => {
                      const newPath = await pickDirectory(settings.flatPath).catch(() => null);
                      if (newPath) updateSettings({ flatPath: newPath });
                    }}
                    className="w-full bg-black/20 border border-white/10 rounded-lg pl-3 pr-8 py-2 text-xs font-mono text-white cursor-pointer hover:bg-black/30 focus:outline-none focus:border-white/50 focus:ring-1 focus:ring-white/50 transition-colors"
                  />
                  <FolderOpen className="w-3.5 h-3.5 absolute right-2.5 top-1/2 -translate-y-1/2 text-white/40 pointer-events-none" />
                </div>
              </div>
            )}

            {settings.directoryMode === 'categorized' && (
              <div className="flex flex-col gap-4">
                <label className="block text-[10px] uppercase font-bold text-white/80 tracking-widest">
                  {t.categorizedRoot}
                </label>
                <div className="relative mb-4">
                  <input
                    type="text"
                    readOnly
                    value={settings.categorizedRoot}
                    onClick={async () => {
                      const newPath = await pickDirectory(settings.categorizedRoot).catch(() => null);
                      if (newPath) updateSettings({ categorizedRoot: newPath });
                    }}
                    className="w-full bg-white text-blue-600 border border-transparent rounded-lg pl-3 pr-8 py-2 text-xs font-black font-mono cursor-pointer shadow-lg hover:bg-neutral-100 transition-colors"
                  />
                  <FolderOpen className="w-3.5 h-3.5 absolute right-2.5 top-1/2 -translate-y-1/2 text-blue-600/50 pointer-events-none" />
                </div>

                <div className="grid grid-cols-2 gap-4">
                  {(['video', 'audio', 'image', 'document', 'archive', 'program', 'other'] as const).map(cat => (
                    <div key={cat} className="flex flex-col gap-1.5 opacity-50">
                      <span className="text-[10px] uppercase font-bold text-white/80">
                        {t[{
                          video: 'categoryVideo',
                          audio: 'categoryAudio',
                          image: 'categoryImage',
                          document: 'categoryDocument',
                          archive: 'categoryArchive',
                          program: 'categoryProgram',
                          other: 'categoryOther'
                        }[cat] as keyof typeof t]}
                      </span>
                      <input
                        type="text"
                        readOnly
                        value={settings.categorizedRoot.replace(/[/\\]$/, '') + (settings.categorizedRoot.includes('\\') ? '\\' : '/') + cat}
                        className="w-full bg-black/10 border border-white/5 rounded-lg px-3 py-2 text-xs font-mono text-white/70 outline-none cursor-default"
                      />
                    </div>
                  ))}
                </div>
              </div>
            )}

            {(settings.directoryMode === 'system_default' || settings.directoryMode === 'custom') && (
              <div className="flex flex-col gap-4">
                <label className="block text-[10px] uppercase font-bold text-white/80 tracking-widest">
                  {t.downloadPaths}
                  {settings.directoryMode === 'system_default' && <span className="ml-2 font-normal text-white/50">{t.readOnlyPath}</span>}
                </label>
                <div className="grid grid-cols-2 gap-4">
                  {(['video', 'audio', 'image', 'document', 'archive', 'program', 'other'] as const).map(cat => {
                    const i18nKey = {
                      video: 'categoryVideo',
                      audio: 'categoryAudio',
                      image: 'categoryImage',
                      document: 'categoryDocument',
                      archive: 'categoryArchive',
                      program: 'categoryProgram',
                      other: 'categoryOther'
                    }[cat] as keyof typeof t;

                    const isSystem = settings.directoryMode === 'system_default';
                    const pathValue = isSystem ? settings.systemPaths[cat] : settings.customPaths[cat];

                    return (
                      <div key={cat} className="flex flex-col gap-1.5">
                        <span className="text-[10px] uppercase font-bold text-white/60">{t[i18nKey]}</span>
                        <div className="relative">
                          <input
                            type="text"
                            readOnly
                            value={pathValue}
                            onClick={async () => {
                              // If system default, clicking to edit will transition to custom
                              const currentPath = isSystem ? settings.systemPaths[cat] : settings.customPaths[cat];
                              const newPath = await pickDirectory(currentPath).catch(() => null);

                              if (newPath) {
                                if (isSystem) {
                                  // Switch mode to custom and pre-fill custom paths with current system paths + this one change
                                  updateSettings({
                                    directoryMode: 'custom',
                                    customPaths: {
                                      ...settings.systemPaths,
                                      [cat]: newPath
                                    }
                                  });
                                } else {
                                  updateSettings({ customPaths: { [cat]: newPath } });
                                }
                              }
                            }}
                            className={cn(
                              "w-full rounded-lg pl-3 pr-8 py-2 text-xs font-mono focus:outline-none transition-colors cursor-pointer",
                              isSystem
                                ? "bg-black/10 border border-white/5 text-white/80 hover:bg-white/10"
                                : "bg-black/20 border border-white/10 text-white hover:bg-black/30 focus:border-white/50 focus:ring-1 focus:ring-white/50"
                            )}
                          />
                          <FolderOpen className={cn("w-3.5 h-3.5 absolute right-2.5 top-1/2 -translate-y-1/2 pointer-events-none", isSystem ? "text-white/30" : "text-white/40")} />
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>
            )}
          </div>

          <div className="pt-4 flex justify-end">
             <button onClick={() => setSettingsModalOpen(false)} className="bg-white text-black text-[10px] font-black uppercase px-6 py-3 rounded-lg tracking-widest hover:bg-neutral-200 transition-colors">
               {t.confirm}
             </button>
          </div>
        </div>
      </div>
    </ModalOverlay>
  );
}
