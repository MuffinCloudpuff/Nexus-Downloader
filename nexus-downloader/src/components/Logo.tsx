import { useApp } from "../store";
import { cn } from "../lib/utils";
import { motion } from "motion/react";

export function Logo({ className }: { className?: string }) {
  const { appLoaded } = useApp();

  return (
    <div className={cn("flex items-center gap-1", className)}>
      {appLoaded ? (
        <>
          <motion.div
            layoutId="logo-container"
            transition={{ type: "spring", stiffness: 400, damping: 30 }}
            className="relative flex items-center justify-center w-12 h-12 shrink-0 group z-10"
          >
            <img
              src="/logo.png"
              alt="Nexus Downloader"
              className="w-full h-full object-contain pointer-events-none"
            />
          </motion.div>

          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            transition={{ duration: 0.5, delay: 0.3 }}
            className="flex flex-col pt-1"
          >
            <h1 aria-label="Nexus Downloader" className="text-xl font-black tracking-tight leading-tight">
              Nexus
              <span className="block text-sm font-semibold tracking-wide text-blue-500">Downloader</span>
            </h1>
          </motion.div>
        </>
      ) : (
        <div className="w-12 h-12 shrink-0" />
      )}
    </div>
  );
}
