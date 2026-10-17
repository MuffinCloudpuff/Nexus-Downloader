/**
 * @license
 * SPDX-License-Identifier: Apache-2.0
 */

import { AppProvider, useApp } from "./store";
import { Sidebar } from "./components/Sidebar";
import { Header } from "./components/Header";
import { DownloadList } from "./components/DownloadList";
import { Modals } from "./components/Modals";
import { SplashScreen } from "./components/SplashScreen";
import { cn } from "./lib/utils";
import { DaemonTaskSync } from "./effects/daemon-task-sync";
import { DesktopShellSync } from "./effects/desktop-shell-sync";
import { AnimatePresence, LayoutGroup } from "motion/react";

function MainLayout() {
  const { settings, appLoaded } = useApp();
  return (
    <LayoutGroup>
      <AnimatePresence>{!appLoaded && <SplashScreen key="splash" />}</AnimatePresence>
      <div className={cn(
        "h-screen w-full overflow-auto font-sans transition-colors duration-300",
        settings.theme === 'dark' ? "dark" : ""
      )}>
        <div className="flex min-w-[1000px] min-h-[680px] h-full bg-[#f3f4f6] text-[#0A0A0B] dark:bg-[#0A0A0B] dark:text-white relative">
          <Sidebar />

          <main className="flex-1 flex flex-col relative z-10 min-w-[744px] bg-black/[0.02] dark:bg-black/40">
            <Header />
            <DownloadList />
          </main>

          <Modals />
          <DaemonTaskSync />
          <DesktopShellSync />
        </div>
      </div>
    </LayoutGroup>
  );
}

export default function App() {
  return (
    <AppProvider>
      <MainLayout />
    </AppProvider>
  );
}
