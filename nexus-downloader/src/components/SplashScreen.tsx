import { useEffect } from "react";
import { motion } from "motion/react";
import { useApp } from "../store";

const SPLASH_DURATION_MS = 2000;

export function SplashScreen() {
  const { setAppLoaded, settings } = useApp();

  useEffect(() => {
    const loadTimer = window.setTimeout(() => {
      setAppLoaded(true);
    }, SPLASH_DURATION_MS);

    return () => {
      window.clearTimeout(loadTimer);
    };
  }, [setAppLoaded]);

  return (
    <motion.div className="fixed inset-0 z-50 flex items-center justify-center pointer-events-none">
      <motion.div
        initial={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.6, ease: "easeInOut" }}
        className={`absolute inset-0 pointer-events-auto ${
          settings.theme === "dark" ? "bg-[#0A0A0B]" : "bg-[#f3f4f6]"
        }`}
      />

      <motion.div
        layoutId="logo-container"
        transition={{ type: "spring", stiffness: 400, damping: 30 }}
        className="flex items-center justify-center shrink-0 w-[50vmin] h-[50vmin] max-w-[500px] max-h-[500px]"
      >
        <motion.div
          initial={{ clipPath: "polygon(0 0, 0 0, 0 100%, 0% 100%)" }}
          animate={{ clipPath: "polygon(0 0, 100% 0, 100% 100%, 0 100%)" }}
          transition={{ duration: 1.2, ease: [0.22, 1, 0.36, 1], delay: 0.2 }}
          className="w-full h-full pointer-events-none"
        >
          <img src="/logo.png" alt="Nexus Downloader" className="w-full h-full object-contain pointer-events-none" />
        </motion.div>
      </motion.div>
    </motion.div>
  );
}
