import { useEffect } from "react";
import { syncCloseBehavior } from "../platform/desktop-shell";
import { useApp } from "../store";

export function DesktopShellSync() {
  const { settings } = useApp();

  useEffect(() => {
    void syncCloseBehavior(settings.closeBehavior === "minimize" ? "hide_to_tray" : "exit_app");
  }, [settings.closeBehavior]);

  return null;
}
