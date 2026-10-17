import { motion, AnimatePresence } from "motion/react";

export function ModalOverlay({ children, isOpen, onClose, className }: { children: React.ReactNode; isOpen: boolean; onClose: () => void; className?: string }) {
  return (
    <AnimatePresence>
      {isOpen && (
        <>
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={onClose}
            className="fixed inset-0 bg-black/40 dark:bg-black/80 backdrop-blur-sm z-50 flex items-center justify-center p-4"
          >
            <motion.div
              initial={{ opacity: 0, scale: 0.95, y: 20 }}
              animate={{ opacity: 1, scale: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.95, y: 20 }}
              transition={{ type: "spring", damping: 25, stiffness: 300 }}
              onClick={(e) => e.stopPropagation()}
              className={className || "w-full max-w-md bg-white text-black dark:bg-[#0A0A0B] dark:text-white border border-black/10 dark:border-white/10 rounded-3xl shadow-2xl overflow-hidden relative font-sans"}
            >
              {children}
            </motion.div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}
