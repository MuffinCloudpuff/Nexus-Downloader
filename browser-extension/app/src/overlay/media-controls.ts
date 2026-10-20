/** Interaction rules stay independent of DOM rendering and background messaging. */
export function taskActionForOverlay(state: {taskId?: string; phase?: string} | null | undefined): "pause" | "resume" | null {
  if (!state?.taskId) return null;
  if (state.phase === "paused") return "resume";
  return ["queued", "starting", "parsing", "resuming", "downloading"].includes(state.phase || "") ? "pause" : null;
}

type PlayerRect = {left: number; right: number; top: number; bottom: number; width: number; height: number};

export function fullscreenOverlayMode(rect: PlayerRect, viewportWidth: number, viewportHeight: number,
  nativeFullscreen: boolean, hostname: string): boolean {
  if (nativeFullscreen || (rect.width >= viewportWidth * 0.98 && rect.height >= viewportHeight * 0.98)) return true;
  const visibleHeight = Math.min(rect.bottom, viewportHeight) - Math.max(rect.top, 0);
  return /(^|\.)douyin\.com$/i.test(hostname) && visibleHeight >= viewportHeight * 0.85 && rect.width >= viewportWidth * 0.25;
}

export function overlayPosition(rect: PlayerRect, width: number, height: number,
  viewportWidth: number, viewportHeight: number, fullscreen = false): {left: number; top: number} {
  const gap = 12;
  // Preserve the established windowed Bilibili/YouTube placement.
  // Fullscreen-style behavior does not mean the video occupies the whole page:
  // Douyin can reserve a sidebar for comments. Anchor inside the visible player.
  let left = fullscreen ? Math.min(rect.right, viewportWidth) - width - gap : rect.right + gap;
  let top = fullscreen ? Math.max(rect.top, 0) + gap : rect.top + gap;
  if (!fullscreen && left + width > viewportWidth - gap) {
    left = Math.min(viewportWidth - width - gap, Math.max(gap, rect.right - width));
    top = rect.top - height - gap;
  }
  if (!fullscreen && top < gap && rect.left - width - gap >= gap) {
    left = rect.left - width - gap;
    top = rect.top;
  }
  if (!fullscreen && top < gap) top = rect.bottom + gap;
  return {
    left: Math.max(gap, Math.min(left, viewportWidth - width - gap)),
    top: Math.max(gap, Math.min(top, viewportHeight - height - gap)),
  };
}

/** Include open-shadow media and Bilibili's player surface when <video> is hidden. */
export function mediaCandidates(scope: Document | ShadowRoot = document, pageUrl = new URL(location.href)): Element[] {
  const candidates = Array.from(scope.querySelectorAll("video, bwp-video"));
  for (const element of Array.from(scope.querySelectorAll("*"))) {
    if (element.id !== "ghostDownloaderMediaDownload" && element.shadowRoot) {
      candidates.push(...mediaCandidates(element.shadowRoot, pageUrl));
    }
  }
  if (scope instanceof Document && /(^|\.)bilibili\.com$/i.test(pageUrl.hostname)
    && (/\/video\/(?:BV|av)/i.test(pageUrl.pathname) || pageUrl.hostname === "player.bilibili.com")) {
    // Prefer the video area; the outer player includes the control bar.
    const surface = scope.querySelector(".bpx-player-video-area")
      || scope.querySelector(".bpx-player-container, #bilibili-player");
    if (surface && !candidates.some(media => {
      const rect = media.getBoundingClientRect();
      return surface.contains(media) && rect.width >= 120 && rect.height >= 80;
    })) candidates.push(surface);
  }
  return candidates;
}
