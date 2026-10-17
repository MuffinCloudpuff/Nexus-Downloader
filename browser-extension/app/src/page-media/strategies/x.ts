import {hostEndsWith, isStreamUrl} from "../url-classify";
import {newestBy, postBindResources} from "../strategy";
import type {MediaStrategy, ResolveContext} from "../strategy";
import type {Resolution} from "../types";

const TWITTER_MEDIA_ID = /\/(?:amplify_video(?:_thumb)?|ext_tw_video(?:_thumb)?|tweet_video(?:_thumb)?)\/(\d+)\//i;

function mediaIdOf(url: string): string {
  return TWITTER_MEDIA_ID.exec(url)?.[1] ?? "";
}

// Master playlist sits one segment under /pl/; variants nest a codec sub-segment
// (avc1/hevc/av01/mp4a). Variants are single-track downloads.
function isMasterTwitterPlaylist(url: string): boolean {
  try {
    const parts = new URL(url).pathname.split("/").filter(Boolean);
    const plIdx = parts.indexOf("pl");
    return plIdx !== -1 && plIdx === parts.length - 2;
  } catch {
    return false;
  }
}

// Master m3u8 only — variants are single-track and dropping one would give silent video.
// Media ids from poster/src pin to this tweet so adjacent tweets' streams don't leak in.
export class XStrategy implements MediaStrategy {
  readonly id = "x";

  matches(pageUrl: URL): boolean {
    const host = pageUrl.hostname;
    return host === "x.com" || host.endsWith(".x.com") || host === "twitter.com" || host.endsWith(".twitter.com");
  }

  resolve(ctx: ResolveContext): Resolution {
    const hintedMediaId = mediaIdOf(ctx.hints.poster) || mediaIdOf(ctx.hints.src);
    const clickedResources = postBindResources(ctx.clicked);
    const allStreams = clickedResources.filter((r) =>
      hostEndsWith(r.url, "video.twimg.com")
      && isStreamUrl(r.url, r.contentType),
    );

    const sessionMediaIds = new Set(
      clickedResources
        .filter((r) => hostEndsWith(r.url, "video.twimg.com"))
        .map((r) => mediaIdOf(r.url))
        .filter(Boolean),
    );
    const singleSessionMediaId = sessionMediaIds.size === 1 ? [...sessionMediaIds][0] : "";

    const candidates = allStreams.filter((r) => {
      const mediaId = mediaIdOf(r.url);
      if (hintedMediaId) { return mediaId === hintedMediaId; }
      if (singleSessionMediaId) { return mediaId === singleSessionMediaId; }
      return r.lockedByMse;
    });

    const masters = candidates.filter((r) => isMasterTwitterPlaylist(r.url));
    const pick = newestBy(masters, (r) => r.capturedAt);
    if (pick) {
      return { kind: "selection", selection: { kind: "stream", url: pick.url } };
    }

    const targetMediaId = hintedMediaId || singleSessionMediaId;
    if (targetMediaId) {
      return { kind: "selection", selection: { kind: "stream_by_media_id", mediaId: targetMediaId } };
    }

    if (ctx.fallbackPhase) {
      const recentMasters = ctx.findRecentResources((url, contentType) =>
        hostEndsWith(url, "video.twimg.com")
        && isStreamUrl(url, contentType)
        && isMasterTwitterPlaylist(url),
      );

      const recentCandidates = recentMasters.filter((r) => {
        const mediaId = mediaIdOf(r.url);
        if (hintedMediaId) { return mediaId === hintedMediaId; }
        if (singleSessionMediaId) { return mediaId === singleSessionMediaId; }
        if (r.sessionId === ctx.clicked.id) { return true; }
        return ctx.clicked.discriminators.has(mediaId);
      });

      const fallbackPick = newestBy(recentCandidates, (r) => r.capturedAt);
      if (fallbackPick) {
        return { kind: "selection", selection: { kind: "stream", url: fallbackPick.url } };
      }
    }
    return {
      kind: "refused",
      message: "无法确认当前 X 视频资源，请先播放目标视频，再点击“下载此媒体”重试",
      diagnostics: {
        clickedResourceCount: clickedResources.length,
        streamCount: allStreams.length,
        sessionDiscriminatorCount: ctx.clicked.discriminators.size,
      },
    };
  }
}
