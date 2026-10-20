import type {MediaStrategy, ResolveContext} from "../strategy";
import type {Resolution} from "../types";

export class WhitelistStrategy implements MediaStrategy {
  readonly id = "whitelist";

  matches(pageUrl: URL): boolean {
    const host = pageUrl.hostname;
    return /(^|\.)bilibili\.com$/.test(host) || /(^|\.)youtube\.com$/.test(host) || host === "youtu.be";
  }

  resolve(ctx: ResolveContext): Resolution {
    let url = ctx.pageUrl.href;
    if (ctx.pageUrl.hostname === "player.bilibili.com") {
      const bvid = ctx.pageUrl.searchParams.get("bvid");
      const id = bvid && /^BV[A-Za-z0-9]+$/.test(bvid) ? bvid : null;
      if (!id) return {kind: "refused", message: "嵌入播放器缺少 BV 编号，请打开 B 站原视频页面后下载"};
      const canonical = new URL(`https://www.bilibili.com/video/${id}/`);
      const page = ctx.pageUrl.searchParams.get("p") || ctx.pageUrl.searchParams.get("page");
      if (page && /^\d+$/.test(page)) canonical.searchParams.set("p", page);
      url = canonical.href;
    }
    // API-Driven Whitelist: Completely bypass media sniffing.
    // We instantly return the current page URL, delegating the actual media parsing
    // and merging entirely to the powerful Rust backend (yt-dlp / BiliApiParser).
    return {
      kind: "selection",
      selection: {
        kind: "single",
        url,
        formKind: "unknown",
      },
    };
  }
}
