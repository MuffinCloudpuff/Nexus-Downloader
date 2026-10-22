import {makeStyles} from "@fluentui/react-components";
import {useRef, useState} from "react";

import type {VisiblePopupView, ThemePreference} from "../shared/types";
import {AdvancedPage} from "./components/AdvancedPage";
import {DownloadsPage} from "./components/DownloadsPage";
import {Header} from "./components/Header";
import {SettingsPage} from "./components/SettingsPage";
import {ToastHost} from "./components/ToastHost";
import {usePopupBridge} from "./hooks/usePopupBridge";
import {DownloadProgressPage} from "./components/DownloadProgressPage";

const useStyles = makeStyles({
  root: {
    position: "relative",
    display: "flex",
    width: "100%",
    height: "100%",
    flexDirection: "column",
    overflow: "hidden",
  },
  content: {
    flex: 1,
    overflowY: "auto",
  },
  // Toolbar progress panels size to their card, including wrapped actions.
  // Keep this out of the fixed-height full-history layout above.
  progressRoot: {
    position: "relative",
    width: "100%",
    height: "auto",
  },
});

export function App({
  themePreference,
  onThemePreferenceChange,
  progressTaskId = null,
}: {
  themePreference: ThemePreference;
  onThemePreferenceChange: (nextPreference: ThemePreference) => void;
  progressTaskId?: string | null;
}) {
  const styles = useStyles();
  const [currentView, setCurrentView] = useState<VisiblePopupView>("tasks");
  const bridge = usePopupBridge(currentView);
  const currentProgressTaskId = useRef(progressTaskId);
  currentProgressTaskId.current = progressTaskId;

  if (progressTaskId) return <div className={styles.progressRoot}>
    <main>
      <DownloadProgressPage taskId={progressTaskId} tasks={bridge.sortedTasks}
        connectionState={bridge.connectionState} connectionMessage={bridge.connectionMessage}
        busy={bridge.isTaskBusy(progressTaskId)}
        onAction={async action => {
          const success = await bridge.performTaskAction(progressTaskId, action);
          if (action === "cancel" && success && currentProgressTaskId.current === progressTaskId) window.close();
        }} />
    </main>
    <ToastHost message={bridge.flashMessage} tone={bridge.flashTone} />
  </div>;

  return (
    <div className={styles.root}>
      <Header
        currentView={currentView}
        connectionState={bridge.connectionState}
        connectionMessage={bridge.connectionMessage}
        mediaDownloadOverlayEnabled={bridge.mediaDownloadOverlayEnabled}
        mediaDownloadOverlayBusy={bridge.isUpdatingMediaDownloadOverlay}
        interceptEnabled={bridge.interceptDownloads}
        interceptBusy={bridge.isUpdatingIntercept}
        onViewChange={setCurrentView}
        onMediaDownloadOverlayToggle={(enabled) => void bridge.setMediaDownloadOverlay(enabled)}
        onInterceptToggle={(enabled) => void bridge.setInterceptDownloads(enabled)}
      />

      <main className={styles.content}>
        {currentView === "tasks" ? (
          <DownloadsPage
            tasks={bridge.sortedTasks}
            isTaskBusy={bridge.isTaskBusy}
            onTaskAction={(taskId, action) => void bridge.performTaskAction(taskId, action)}
          />
        ) : null}

        {currentView === "advanced" ? (
          <AdvancedPage
            featureStates={bridge.featureStates}
            isFeatureBusy={bridge.isFeatureBusy}
            onFeatureToggle={(feature) => void bridge.toggleFeature(feature)}
            mediaItems={bridge.mediaItems}
            mediaPlaybackState={bridge.mediaPlaybackState}
            mediaBusy={bridge.isUpdatingMedia}
            onMediaItemChange={(index) => void bridge.setMediaIndex(index)}
            onMediaAction={(action, value) => void bridge.performMediaAction(action, value)}
          />
        ) : null}

        {currentView === "settings" ? (
          <SettingsPage
            desktopVersion={bridge.desktopVersion}
            token={bridge.token}
            serverUrl={bridge.serverUrl}
            savingToken={bridge.isSavingToken}
            savingServerUrl={bridge.isSavingServerUrl}
            refreshingConnection={bridge.isRefreshingConnection}
            requestingPairing={bridge.isRequestingPairing}
            onSaveToken={bridge.saveToken}
            onSaveServerUrl={bridge.saveServerUrl}
            onRefreshConnection={bridge.refreshConnection}
            onRequestPairing={bridge.requestPairing}
            themePreference={themePreference}
            onThemePreferenceChange={onThemePreferenceChange}
          />
        ) : null}
      </main>

      <ToastHost message={bridge.flashMessage} tone={bridge.flashTone} />
    </div>
  );
}
