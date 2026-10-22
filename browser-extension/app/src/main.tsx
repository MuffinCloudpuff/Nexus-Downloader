import {FluentProvider} from "@fluentui/react-components";
import {StrictMode, useEffect, useState} from "react";
import {createRoot} from "react-dom/client";

import {App} from "./popup/App";
import {useThemePreference} from "./popup/hooks/useThemePreference";
import "./index.css";
import {progressTaskId} from "./shared/progress-popup";

const initialTaskId = progressTaskId(window.location.search);
if (initialTaskId) {
  document.documentElement.dataset.progressPopup = "true";
  document.title = "本次下载 · Nexus Downloader";
}

function PopupRoot() {
  const { theme, themePreference, setThemePreference } = useThemePreference();
  const [focusedTaskId, setFocusedTaskId] = useState(initialTaskId);

  useEffect(() => {
    // Only an automatically opened progress panel follows the next download.
    // A manually opened full-history panel must keep its normal behavior.
    if (!initialTaskId) return;
    const focusTask = (message: unknown, sender: chrome.runtime.MessageSender, reply: (response: unknown) => void) => {
      if (sender.id !== chrome.runtime.id || !message || typeof message !== "object") return;
      const request = message as {type?: unknown; taskId?: unknown};
      if (request.type !== "progress_popup_focus" || typeof request.taskId !== "string" || !request.taskId.trim()) return;
      setFocusedTaskId(request.taskId);
      reply({focused: true});
    };
    chrome.runtime.onMessage.addListener(focusTask);
    return () => chrome.runtime.onMessage.removeListener(focusTask);
  }, []);

  return (
    <FluentProvider className="gd4b-provider" theme={theme}>
      <App
        progressTaskId={focusedTaskId}
        themePreference={themePreference}
        onThemePreferenceChange={(nextPreference) => void setThemePreference(nextPreference)}
      />
    </FluentProvider>
  );
}

createRoot(document.getElementById("app") as HTMLElement).render(
  <StrictMode>
    <PopupRoot />
  </StrictMode>,
);
