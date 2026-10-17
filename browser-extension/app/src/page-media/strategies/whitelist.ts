import type {MediaStrategy, ResolveContext} from "../strategy";
import type {Resolution} from "../types";

export class WhitelistStrategy implements MediaStrategy {
  readonly id = "whitelist";

  matches(pageUrl: URL): boolean {
    const host = pageUrl.hostname;
    return host.endsWith("bilibili.com") || host.endsWith("youtube.com") || host === "youtu.be";
  }

  resolve(ctx: ResolveContext): Resolution {
    // API-Driven Whitelist: Completely bypass media sniffing.
    // We instantly return the current page URL, delegating the actual media parsing
    // and merging entirely to the powerful Rust backend (yt-dlp / BiliApiParser).
    return {
      kind: "selection",
      selection: {
        kind: "single",
        url: ctx.pageUrl.href,
        formKind: "unknown",
      },
    };
  }
}
