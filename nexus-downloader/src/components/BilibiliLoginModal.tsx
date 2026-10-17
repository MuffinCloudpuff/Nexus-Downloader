import { useEffect, useState } from "react";
import { useApp } from "../store";
import { generateBilibiliQr, getBilibiliQrStatus } from "../api/daemon-client";
import { ModalOverlay } from "./ModalOverlay";

export function BilibiliLoginModal() {
  const { isBilibiliModalOpen, setBilibiliModalOpen, t } = useApp();
  const [qrImage, setQrImage] = useState<string | null>(null);
  const [sessionKey, setSessionKey] = useState<string | null>(null);
  const [message, setMessage] = useState(t.qrValid);

  useEffect(() => {
    if (!isBilibiliModalOpen) {
      return;
    }
    let cancelled = false;

    async function loadQr() {
      setQrImage(null);
      setSessionKey(null);
      setMessage(t.qrValid);
      try {
        const response = await generateBilibiliQr();
        if (cancelled) {
          return;
        }
        setQrImage(response.qr_image_data_url || response.qr_image_url || null);
        setSessionKey(response.session_key || null);
        if (response.error) {
          setMessage(response.error);
        }
      } catch (error) {
        if (!cancelled) {
          setMessage(error instanceof Error ? error.message : t.statusFailed);
        }
      }
    }

    void loadQr();
    return () => {
      cancelled = true;
    };
  }, [isBilibiliModalOpen, t.qrValid, t.statusFailed]);

  useEffect(() => {
    if (!isBilibiliModalOpen || !sessionKey) {
      return;
    }
    let cancelled = false;

    async function pollStatus() {
      try {
        const status = await getBilibiliQrStatus(sessionKey);
        if (!cancelled) {
          setMessage(status.message || status.login_state);
        }
      } catch (error) {
        if (!cancelled) {
          setMessage(error instanceof Error ? error.message : t.statusFailed);
        }
      }
    }

    const interval = window.setInterval(() => void pollStatus(), 2000);
    void pollStatus();
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [isBilibiliModalOpen, sessionKey, t.statusFailed]);

  return (
    <ModalOverlay isOpen={isBilibiliModalOpen} onClose={() => setBilibiliModalOpen(false)}>
      <div className="p-8 flex flex-col items-center text-center">
        <img src="/bilibili.png" alt="Bilibili" className="w-16 h-16 rounded-2xl mb-6 shadow-sm object-cover" />
        <h2 className="text-2xl font-black italic tracking-tighter mb-2 uppercase">{t.bilibiliLoginTitle}</h2>
        <p className="text-[10px] uppercase tracking-widest text-black/50 dark:text-white/50 mb-8 font-bold">{t.scanQr}</p>

        <div className="w-48 h-48 bg-white rounded-xl p-2 mb-6 shadow-inner border border-black/10 dark:border-0 relative overflow-hidden group">
          {qrImage ? (
            <img src={qrImage} alt="Bilibili QR" className="w-full h-full object-contain" />
          ) : (
            <div className="w-full h-full flex items-center justify-center text-xs font-black text-black/40">
              ...
            </div>
          )}
        </div>

        <p className="text-xs font-mono text-black/40 dark:text-white/40">{message}</p>
      </div>
    </ModalOverlay>
  );
}
