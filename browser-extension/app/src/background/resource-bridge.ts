import type {CapturedResource, DesktopRequestResult, ResourceCollectionState} from "../shared/types";
import type {
    ClientTelemetryEvent,
    CapturedResourcePayload,
    CreateTaskHttpReq,
    CreateTaskSource,
} from "../shared/protocol/daemon";
import {
    canUseOnlineMergeSelection,
    domainFromUrl,
    fileExtension,
    filenameFromUrl,
    mimeFromUrl,
    sortResourcesForOnlineMerge,
} from "../shared/utils";
import {isCatCatchMedia} from "../shared/cat-catch";
import type {Selection} from "../page-media/types";
import {
    BRIDGE_HEADER_SNAPSHOTS_KEY,
    BRIDGE_LAST_ACTIVE_TAB_KEY,
    BRIDGE_PERSIST_DEBOUNCE_MS,
    BRIDGE_RESOURCE_CACHE_KEY,
    HEADER_EXPIRATION_MS,
    HEADER_SNAPSHOT_LIMIT,
    PROTOCOL_VERSION,
    RESOURCE_LIMIT,
} from "./constants";
import {bridgeStorageSet, findTab, loadFromBridgeStorage, queryTabs,} from "./chrome-helpers";
import { encryptCookies, getDomainCookies, getUrlCookies } from "./crypto";
import {isAbsoluteDownloadPath} from "./browser-download-target";

type HeaderSnapshot = {
  url: string;
  headers: Record<string, string>;
  capturedAt: number;
  tabId: number | null;
  supportsRange: boolean;
};

type HeaderSnapshotMatch = {
  snapshot: HeaderSnapshot;
  strategy: "exact" | "normalized" | "same_path";
};

// From cat-catch's addMedia path — DOM-side discovery pushed up by capturePageResource.
type CapturePayload = {
  url: string;
  href?: string;
  filename?: string;
  mime?: string;
  ext?: string;
  poster?: string;
  resourceUrls?: string[];
  requestHeaders?: Record<string, string>;
};

// From overlay click — strategy already disambiguated against the right <video>, so
// background just augments with headers and dispatches.
type ClickPayload = {
  selection: Selection;
  href: string;
  title: string;
  traceId?: string;
};

type NetworkResponseMeta = {
  size: number;
  mime: string;
  filename: string;
  supportsRange: boolean;
};

type CreateTaskSender = (request: CreateTaskHttpReq) => Promise<DesktopRequestResult>;
type TelemetryRecorder = (event: ClientTelemetryEvent) => void;
type ResourceBucket = Map<string, CapturedResource>;
type ResourceMatchAwaiter = {
  predicate: (resource: CapturedResource) => boolean;
  resolve: (resource: CapturedResource | null) => void;
  timer: ReturnType<typeof setTimeout>;
};
type HeaderSnapshotAwaiter = {
  urls: string[];
  tabId: number | null;
  resolve: (match: HeaderSnapshotMatch | null) => void;
  timer: number;
};

const HEADER_WHITELIST = new Set([
  "accept",
  "accept-language",
  "authorization",
  "cookie",
  "origin",
  "referer",
  "sec-ch-ua",
  "sec-ch-ua-arch",
  "sec-ch-ua-bitness",
  "sec-ch-ua-full-version",
  "sec-ch-ua-full-version-list",
  "sec-ch-ua-mobile",
  "sec-ch-ua-model",
  "sec-ch-ua-platform",
  "sec-ch-ua-platform-version",
  "sec-fetch-dest",
  "sec-fetch-mode",
  "sec-fetch-site",
  "user-agent",
  "priority",
]);

const MIME_EXTENSIONS: Record<string, string> = {
  "application/dash+xml": "mpd",
  "application/mpegurl": "m3u8",
  "application/vnd.apple.mpegurl": "m3u8",
  "application/x-mpegurl": "m3u8",
  "audio/aac": "aac",
  "audio/flac": "flac",
  "audio/mp4": "m4a",
  "audio/mpeg": "mp3",
  "audio/ogg": "ogg",
  "audio/wav": "wav",
  "audio/webm": "webm",
  "video/mp2t": "ts",
  "video/mp4": "mp4",
  "video/quicktime": "mov",
  "video/webm": "webm",
  "video/x-flv": "flv",
  "video/x-m4v": "m4v",
  "video/x-ms-wmv": "wmv",
};

export function createResourceBridge(options: {
  createTask: CreateTaskSender;
  startBrowserTask?: (id: string) => Promise<void>;
  recordTelemetry?: TelemetryRecorder;
}) {
  let bridgePersistTimer: number | null = null;
  let restoredFromStorage = false;
  let lastActiveTabId: number | null = null;

  const resourceCache = new Map<number, ResourceBucket>();
  const resourcesById = new Map<string, CapturedResource>();
  const headerSnapshotsByUrl = new Map<string, HeaderSnapshot>();

  // Lets resolveResourceForUrl wait when click races ahead of webRequest (SW restart).
  const resourceCacheAwaiters = new Map<string, Set<(resource: CapturedResource) => void>>();
  const resourceMatchAwaiters = new Set<ResourceMatchAwaiter>();
  const headerSnapshotAwaiters = new Set<HeaderSnapshotAwaiter>();

  function hashString(value: string): string {
    let hash = 2166136261;
    for (let index = 0; index < value.length; index += 1) {
      hash ^= value.charCodeAt(index);
      hash = Math.imul(hash, 16777619);
    }
    return (hash >>> 0).toString(16).padStart(8, "0");
  }

  function requestIdFor(source: CreateTaskSource, stableKey: string): string {
    return `req_${source}_${hashString(stableKey || crypto.randomUUID())}`;
  }

  function uniqueRequestIdFor(source: CreateTaskSource, stableKey: string): string {
    const random = typeof crypto?.randomUUID === "function"
      ? crypto.randomUUID()
      : `${Date.now().toString(36)}_${Math.random().toString(36).slice(2)}`;
    return `req_${source}_${hashString(`${stableKey || ""}:${random}`)}`;
  }

  function traceIdFor(surface: string): string {
    const random = typeof crypto?.randomUUID === "function"
      ? crypto.randomUUID()
      : `${Date.now().toString(36)}_${Math.random().toString(36).slice(2)}`;
    return `trace_${surface}_${random}`.replace(/[^a-zA-Z0-9_-]/g, "_").slice(0, 96);
  }

  function clientContext(tabId: number | null | undefined, pageUrl?: string, traceId?: string) {
    return {
      extension_id: chrome.runtime.id,
      tab_id: typeof tabId === "number" && Number.isFinite(tabId) ? tabId : undefined,
      page_url: pageUrl || undefined,
      protocol_version: PROTOCOL_VERSION,
      trace_id: traceId || undefined,
    };
  }

  function recordUiFailure(traceId: string | undefined, action: string, message: string, details?: Record<string, unknown>) {
    options.recordTelemetry?.({
      event_type: "client.ui_state",
      trace_id: traceId,
      surface: "background",
      action,
      status: "failed",
      message,
      details,
    });
  }

  function resourceKindOf(resource: CapturedResource): string {
    const mime = resource.mime.toLowerCase();
    if (mime.startsWith("video/")) {
      return "video";
    }
    if (mime.startsWith("audio/")) {
      return "audio";
    }
    if (resource.url.toLowerCase().includes(".m3u8") || resource.url.toLowerCase().includes(".mpd")) {
      return "stream";
    }
    return "unknown";
  }

  function capturedResourcePayload(resource: CapturedResource): CapturedResourcePayload {
    return {
      kind: resourceKindOf(resource),
      url: resource.url,
      mime: resource.mime || undefined,
      filename: filenameForDesktop(resource),
      size: resource.size || undefined,
      supports_range: resource.supportsRange,
      headers: resource.requestHeaders,
      page_title: resource.pageTitle || undefined,
      page_url: resource.pageUrl || undefined,
    };
  }

  async function encryptedCookiesForUrl(url: string): Promise<{ encrypted_cookies?: string; cookie_iv?: string }> {
    const domain = domainFromUrl(url);
    if (!domain.includes("youtube.com") && !domain.includes("bilibili.com")) {
      return {};
    }

    const rootDomain = domain.includes("youtube.com") ? ".youtube.com" : ".bilibili.com";
    const rawCookies = await getUrlCookies(url) || await getDomainCookies(rootDomain);
    if (!rawCookies) {
      console.warn("[GD3 ResourceBridge] No browser cookies available for resource host", { domain });
      return {};
    }

    const encrypted = await encryptCookies(rawCookies);
    if (!encrypted) {
      return {};
    }

    return {
      encrypted_cookies: encrypted.encrypted,
      cookie_iv: encrypted.iv,
    };
  }

  function notifyResourceCached(resource: CapturedResource): void {
    const key = resource.id;
    const set = resourceCacheAwaiters.get(key);
    if (set && set.size > 0) {
      resourceCacheAwaiters.delete(key);
      for (const resolver of set) {
        try { resolver(resource); } catch { /* swallow */ }
      }
    }

    for (const awaiter of [...resourceMatchAwaiters]) {
      let matched = false;
      try { matched = awaiter.predicate(resource); } catch { matched = false; }
      if (!matched) { continue; }
      resourceMatchAwaiters.delete(awaiter);
      clearTimeout(awaiter.timer);
      awaiter.resolve(resource);
    }
  }

  function awaitResourceCached(key: string, timeoutMs: number): Promise<CapturedResource | null> {
    return new Promise((resolve) => {
      let set = resourceCacheAwaiters.get(key);
      if (!set) { set = new Set(); resourceCacheAwaiters.set(key, set); }
      let settled = false;
      const finish = (value: CapturedResource | null) => {
        if (settled) { return; }
        settled = true;
        set?.delete(resolver);
        if (set && set.size === 0) { resourceCacheAwaiters.delete(key); }
        clearTimeout(timer);
        resolve(value);
      };
      const resolver = (resource: CapturedResource) => finish(resource);
      set.add(resolver);
      const timer = setTimeout(() => finish(null), timeoutMs);
    });
  }

  function awaitResourceMatch(
    predicate: (resource: CapturedResource) => boolean,
    timeoutMs: number,
  ): Promise<CapturedResource | null> {
    return new Promise((resolve) => {
      let awaiter: ResourceMatchAwaiter;
      const finish = (resource: CapturedResource | null) => {
        resourceMatchAwaiters.delete(awaiter);
        clearTimeout(awaiter.timer);
        resolve(resource);
      };
      awaiter = {
        predicate,
        resolve: finish,
        timer: setTimeout(() => finish(null), timeoutMs),
      };
      resourceMatchAwaiters.add(awaiter);
    });
  }

  function selectAllowedHeaders(headers: Record<string, string> | undefined): Record<string, string> {
    const result: Record<string, string> = {};
    for (const [key, value] of Object.entries(headers ?? {})) {
      const name = key.trim().toLowerCase();
      if (!HEADER_WHITELIST.has(name)) { continue; }
      const text = value.trim();
      if (!text) { continue; }
      result[name] = text;
    }
    return result;
  }

  function basenameOf(path: string): string {
    const trimmed = path.trim();
    if (!trimmed) { return ""; }
    const slashIndex = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
    return slashIndex >= 0 ? trimmed.slice(slashIndex + 1) : trimmed;
  }

  function isCapturableUrl(url: string): boolean {
    return /^https?:/i.test(url);
  }

  function sortResources(resources: Iterable<CapturedResource>): CapturedResource[] {
    return [...resources].sort((left, right) => right.capturedAt - left.capturedAt);
  }

  function urlWithoutHash(value: string, allowBlob = false): string {
    if (!value) { return ""; }
    if (allowBlob && value.startsWith("blob:")) { return value; }
    try {
      const url = new URL(value);
      url.hash = "";
      return url.toString();
    } catch {
      return value.split("#", 1)[0] ?? value;
    }
  }

  function comparableUrl(value: string): string {
    if (!value) { return ""; }
    try {
      const parsed = new URL(value);
      parsed.hash = "";
      const noiseParams = [
        "verify",
        "expires",
        "expire",
        "token",
        "signature",
        "sig",
        "download",
        "filename",
      ];
      for (const param of noiseParams) {
        parsed.searchParams.delete(param);
      }
      return parsed.toString();
    } catch {
      return urlWithoutHash(value, true);
    }
  }

  function sameOriginPath(left: string, right: string): boolean {
    try {
      const leftUrl = new URL(left);
      const rightUrl = new URL(right);
      return leftUrl.origin === rightUrl.origin && leftUrl.pathname === rightUrl.pathname;
    } catch {
      return false;
    }
  }

  // Schema round-trips through JSON cleanly; only the header whitelist needs re-running.
  function resourceFromStorage(resource: CapturedResource): CapturedResource {
    return { ...resource, requestHeaders: selectAllowedHeaders(resource.requestHeaders) };
  }

  function headerSnapshotFromStorage(snapshot: HeaderSnapshot): HeaderSnapshot {
    return { ...snapshot, headers: selectAllowedHeaders(snapshot.headers) };
  }

  function responseMeta(headers: chrome.webRequest.HttpHeader[] | undefined): NetworkResponseMeta {
    const meta: NetworkResponseMeta = { size: 0, mime: "", filename: "", supportsRange: false };
    let contentLengthSize = 0;
    let contentRangeSize = 0;
    for (const header of headers ?? []) {
      const name = (header.name ?? "").toLowerCase();
      const value = (header.value ?? "").trim();
      if (name === "content-length") {
        const size = Number.parseInt(value, 10);
        contentLengthSize = Number.isFinite(size) && size > 0 ? size : contentLengthSize;
      } else if (name === "content-type") {
        meta.mime = value.split(";")[0]?.trim().toLowerCase() ?? "";
      } else if (name === "content-disposition") {
        const match = /filename\*\s*=\s*UTF-8''([^;]+)|filename\s*=\s*"?([^";]+)"?/i.exec(value);
        const filename = match?.[1] ?? match?.[2] ?? "";
        try {
          meta.filename = basenameOf(decodeURIComponent(filename));
        } catch {
          meta.filename = basenameOf(filename);
        }
      } else if (name === "accept-ranges") {
        meta.supportsRange = value.toLowerCase().includes("bytes");
      } else if (name === "content-range") {
        const size = Number.parseInt(value.split("/")[1] ?? "", 10);
        contentRangeSize = Number.isFinite(size) && size > 0 ? size : contentRangeSize;
        meta.supportsRange = true;
      }
    }
    meta.size = contentRangeSize || contentLengthSize;
    return meta;
  }

  function shouldCaptureNetworkResource(details: chrome.webRequest.OnResponseStartedDetails, meta: NetworkResponseMeta): boolean {
    if (!isCapturableUrl(details.url)) {
      return false;
    }
    const extension = fileExtension(meta.filename || filenameFromUrl(details.url));
    return details.type === "media" || isCatCatchMedia(extension, meta.mime);
  }

  function shouldCaptureRequestResource(details: chrome.webRequest.OnSendHeadersDetails): boolean {
    if (!isCapturableUrl(details.url)) {
      return false;
    }
    return details.type === "media" || isCatCatchMedia(fileExtension(filenameFromUrl(details.url)), mimeFromUrl(details.url));
  }

  function urlsSharePathPrefix(left: string, right: string): boolean {
    const normalizedLeft = urlWithoutHash(left);
    const normalizedRight = urlWithoutHash(right);
    if (!normalizedLeft || !normalizedRight) {
      return false;
    }
    return normalizedLeft === normalizedRight || normalizedLeft.startsWith(normalizedRight) || normalizedRight.startsWith(normalizedLeft);
  }

  async function resolveTabIdFromPageUrl(pageUrl: string): Promise<number | null> {
    const normalizedPageUrl = urlWithoutHash(pageUrl);
    if (!normalizedPageUrl) {
      return null;
    }

    const tabs = await queryTabs({});
    const exactMatch = tabs.find((tab) => tab.id && tab.url && urlsSharePathPrefix(tab.url, normalizedPageUrl));
    if (exactMatch?.id) {
      return exactMatch.id;
    }

    try {
      const initiatorUrl = new URL(normalizedPageUrl);
      const originMatch = tabs.find((tab) => {
        if (!tab.id || !tab.url) {
          return false;
        }
        try {
          return new URL(tab.url).origin === initiatorUrl.origin;
        } catch {
          return false;
        }
      });
      if (originMatch?.id) {
        return originMatch.id;
      }
    } catch {
      // Ignore invalid page URLs.
    }

    return null;
  }

  async function persistBridgeState() {
    bridgePersistTimer = null;
    pruneHeaderSnapshots();
    await bridgeStorageSet({
      [BRIDGE_RESOURCE_CACHE_KEY]: serializeResourceCache(),
      [BRIDGE_HEADER_SNAPSHOTS_KEY]: [...headerSnapshotsByUrl.values()],
      [BRIDGE_LAST_ACTIVE_TAB_KEY]: lastActiveTabId ?? 0,
    });
  }

  function scheduleBridgeStatePersist() {
    if (bridgePersistTimer !== null) {
      return;
    }
    bridgePersistTimer = self.setTimeout(() => {
      void persistBridgeState();
    }, BRIDGE_PERSIST_DEBOUNCE_MS);
  }

  function serializeResourceCache(): Record<string, CapturedResource[]> {
    const result: Record<string, CapturedResource[]> = {};
    for (const [tabId, bucket] of resourceCache.entries()) {
      result[String(tabId)] = sortResources(bucket.values()).slice(0, RESOURCE_LIMIT);
    }
    return result;
  }

  function pruneHeaderSnapshots() {
    const now = Date.now();
    const snapshots = [...headerSnapshotsByUrl.values()]
      .filter((snapshot) => snapshot.url && now - snapshot.capturedAt <= HEADER_EXPIRATION_MS)
      .sort((left, right) => right.capturedAt - left.capturedAt)
      .slice(0, HEADER_SNAPSHOT_LIMIT);

    headerSnapshotsByUrl.clear();
    for (const snapshot of snapshots) {
      headerSnapshotsByUrl.set(snapshot.url, snapshot);
    }
  }

  function clearResourcesForTab(tabId: number) {
    const bucket = resourceCache.get(tabId);
    if (!bucket) {
      return;
    }
    for (const resourceId of bucket.keys()) {
      resourcesById.delete(resourceId);
    }
    resourceCache.delete(tabId);
    scheduleBridgeStatePersist();
  }

  function clearHeaderSnapshotsForTab(tabId: number) {
    let changed = false;
    for (const [url, snapshot] of headerSnapshotsByUrl.entries()) {
      if (snapshot.tabId === tabId) {
        headerSnapshotsByUrl.delete(url);
        changed = true;
      }
    }
    if (changed) {
      scheduleBridgeStatePersist();
    }
  }

  async function setLastActiveTab(tabId: number | null) {
    if (lastActiveTabId === tabId) {
      return;
    }
    lastActiveTabId = tabId;
    scheduleBridgeStatePersist();
  }

  async function refreshActiveTabFromBrowser(): Promise<number | null> {
    const tabs = await queryTabs({ active: true, currentWindow: true });
    const tabId = tabs[0]?.id ?? null;
    await setLastActiveTab(tabId);
    return tabId;
  }

  async function resolveActiveTabId(preferredTabId: number | null = null): Promise<number | null> {
    if (preferredTabId != null) {
      const preferredTab = await findTab(preferredTabId);
      if (preferredTab?.id) {
        await setLastActiveTab(preferredTab.id);
        return preferredTab.id;
      }
    }

    const activeTabId = await refreshActiveTabFromBrowser();
    if (activeTabId != null) {
      return activeTabId;
    }

    if (lastActiveTabId != null) {
      const current = await findTab(lastActiveTabId);
      if (current?.id) {
        return current.id;
      }
    }

    return null;
  }

  function filenameWithExtension(baseName: string, extension: string): string {
    const trimmedBaseName = cleanFilename(baseName) || "resource";
    const normalizedExt = extension.trim().replace(/^\./, "").toLowerCase();
    if (!normalizedExt) {
      return trimmedBaseName;
    }
    if (fileExtension(trimmedBaseName) === normalizedExt) {
      return trimmedBaseName;
    }
    return `${trimmedBaseName}.${normalizedExt}`;
  }

  function cleanFilename(value?: string): string {
    return (value ?? "")
      .trim()
      .replace(/[<>:"/\\|?*\x00-\x1f]+/g, " ")
      .replace(/\s+/g, " ")
      .replace(/[. ]+$/g, "")
      .trim()
      .slice(0, 160);
  }

  function extensionFromMime(mime?: string): string {
    const type = mime?.split(";")[0]?.trim().toLowerCase() ?? "";
    return MIME_EXTENSIONS[type] ?? "";
  }

  function resolveBridgeFilename(payload: CapturePayload): string {
    const ext = payload.ext?.trim() || extensionFromMime(payload.mime);
    const explicit = cleanFilename(payload.filename);
    if (explicit) {
      return ext && !fileExtension(explicit) ? filenameWithExtension(explicit, ext) : explicit;
    }

    const fromUrl = cleanFilename(filenameFromUrl(payload.url));
    if (fromUrl) {
      return ext ? filenameWithExtension(fromUrl, ext) : fromUrl;
    }

    return ext ? filenameWithExtension("resource", ext) : "resource";
  }

  function filenameForDesktop(resource: CapturedResource): string {
    const urlFilename = filenameFromUrl(resource.url);
    const current = cleanFilename(resource.filename || urlFilename);
    const extension = fileExtension(current)
      || fileExtension(urlFilename)
      || extensionFromMime(resource.mime);
    const title = cleanFilename(resource.pageTitle);
    const baseName = title || current || "resource";
    return filenameWithExtension(baseName, extension);
  }

  function isGenericXPageTitle(title: string): boolean {
    return /^(?:\(\d+\)\s*)?(?:主页|Home)\s+X$/i.test(title);
  }

  function sourcePrefixForResource(resource: CapturedResource): string {
    const domain = domainFromUrl(resource.pageUrl || resource.url).toLowerCase();
    const urlDomain = domainFromUrl(resource.url).toLowerCase();
    if (
      domain.endsWith("x.com")
      || domain.endsWith("twitter.com")
      || urlDomain.endsWith("twimg.com")
      || urlDomain.endsWith("twitter.com")
    ) {
      return "X";
    }
    if (domain.includes("bilibili.com") || urlDomain.includes("bilibili.com")) {
      return "B";
    }
    if (
      domain.includes("youtube.com")
      || domain.includes("youtu.be")
      || urlDomain.includes("youtube.com")
      || urlDomain.includes("googlevideo.com")
    ) {
      return "Y";
    }
    return "";
  }

  function stripExistingSourcePrefix(title: string): string {
    return title.replace(/^(?:X|B|Y)-+/i, "").trim();
  }

  function normalizeSourceTitle(resource: CapturedResource, fallback: string): string {
    const prefix = sourcePrefixForResource(resource);
    const cleanedFallback = stripExistingSourcePrefix(cleanFilename(fallback));
    const base = prefix === "X" && isGenericXPageTitle(cleanedFallback)
      ? ""
      : cleanedFallback;
    if (!prefix) {
      return base || "media";
    }
    return `${prefix}-${base || "media"}`;
  }

  function mediaIdentifierFromUrl(url: string): string {
    try {
      const parsed = new URL(url);
      const twitterMatch = /\/(?:amplify_video(?:_thumb)?|ext_tw_video(?:_thumb)?|tweet_video(?:_thumb)?)\/(\d+)\//i.exec(parsed.pathname);
      if (twitterMatch?.[1]) {
        return twitterMatch[1];
      }
      const pathParts = parsed.pathname.split("/").filter(Boolean);
      const plIndex = pathParts.indexOf("pl");
      if (plIndex > 0) {
        return pathParts[plIndex - 1];
      }
      for (const key of ["__vid", "v", "video_id", "id"]) {
        const value = parsed.searchParams.get(key);
        if (value) {
          return value;
        }
      }
    } catch {
      // Fall through to caller fallback.
    }
    return "";
  }

  function isTwitterMasterPlaylistUrl(url: string): boolean {
    try {
      const parsed = new URL(url);
      if (!parsed.hostname.toLowerCase().endsWith("twimg.com")) {
        return false;
      }
      const parts = parsed.pathname.split("/").filter(Boolean);
      const plIndex = parts.indexOf("pl");
      return plIndex !== -1 && plIndex === parts.length - 2;
    } catch {
      return false;
    }
  }

  function titleForPageMediaResource(resource: CapturedResource): string {
    const pageTitle = cleanFilename(resource.pageTitle);
    const urlName = cleanFilename(filenameFromUrl(resource.url));
    const mediaId = mediaIdentifierFromUrl(resource.url);
    const base = pageTitle && !isGenericXPageTitle(pageTitle)
      ? pageTitle
      : urlName || "media";
    const sourceTitle = normalizeSourceTitle(resource, base);
    return mediaId ? `${sourceTitle}-${mediaId}` : sourceTitle;
  }

  function filenameForPageMediaResource(resource: CapturedResource): string {
    const title = titleForPageMediaResource(resource);
    const urlFilename = filenameFromUrl(resource.url);
    const extension = fileExtension(resource.filename)
      || fileExtension(urlFilename)
      || extensionFromMime(resource.mime);
    return extension ? filenameWithExtension(title, extension) : title;
  }

  async function resolveBridgeResourceTabId(sender: chrome.runtime.MessageSender, href?: string): Promise<number | null> {
    if (sender.tab?.id) {
      await setLastActiveTab(sender.tab.id);
      return sender.tab.id;
    }

    const normalizedHref = href?.trim() ?? "";
    if (normalizedHref) {
      const matchedTabId = await resolveTabIdFromPageUrl(normalizedHref);
      if (matchedTabId != null) {
        return matchedTabId;
      }
    }

    return resolveActiveTabId();
  }

  async function resolveNetworkResourceTabId(details: chrome.webRequest.OnResponseStartedDetails): Promise<number | null> {
    if (details.tabId > 0) {
      return details.tabId;
    }
    const snapshotTabId = resolveHeaderSnapshotMatch(details.url)?.snapshot.tabId;
    if (snapshotTabId != null) {
      return snapshotTabId;
    }
    const matchedTabId = await resolveTabIdFromPageUrl(details.initiator ?? "");
    return matchedTabId ?? resolveActiveTabId();
  }

  function bucketForTab(tabId: number): ResourceBucket {
    const existing = resourceCache.get(tabId);
    if (existing) {
      return existing;
    }

    const bucket = new Map<string, CapturedResource>();
    resourceCache.set(tabId, bucket);
    return bucket;
  }

  function trimBucket(tabId: number) {
    const bucket = resourceCache.get(tabId);
    if (!bucket || bucket.size <= RESOURCE_LIMIT) {
      return;
    }

    const keepIds = new Set(sortResources(bucket.values()).slice(0, RESOURCE_LIMIT).map((resource) => resource.id));
    for (const resourceId of bucket.keys()) {
      if (keepIds.has(resourceId)) {
        continue;
      }
      bucket.delete(resourceId);
      resourcesById.delete(resourceId);
    }
  }

  function cacheResource(resource: CapturedResource) {
    const bucket = bucketForTab(resource.tabId);
    const existing = bucket.get(resource.id);
    const merged: CapturedResource = existing
      ? {
          ...existing,
          ...resource,
          pageTitle: resource.pageTitle || existing.pageTitle,
          pageUrl: resource.pageUrl || existing.pageUrl,
          filename: resource.filename || existing.filename,
          mime: resource.mime || existing.mime,
          size: resource.size > 0 ? resource.size : existing.size,
          supportsRange: resource.supportsRange || existing.supportsRange,
          referer: resource.referer || existing.referer,
          // Fresh cookies/auth/sec-* override stale ones (per-key, like every field above).
          requestHeaders: {
            ...existing.requestHeaders,
            ...resource.requestHeaders,
          },
          capturedAt: Math.max(existing.capturedAt, resource.capturedAt),
          sentToDesktopAt: existing.sentToDesktopAt ?? resource.sentToDesktopAt,
        }
      : resource;

    bucket.set(merged.id, merged);
    resourcesById.set(merged.id, merged);
    notifyResourceCached(merged);
    const mergedUrl = urlWithoutHash(merged.url, true);
    if (mergedUrl && (merged.size > 0 || merged.mime || merged.supportsRange)) {
      for (const resource of resourcesById.values()) {
        if (resource.id === merged.id || !resource.id.endsWith(`:${mergedUrl}`)) {
          continue;
        }
        resource.size = merged.size > 0 ? merged.size : resource.size;
        resource.mime = merged.mime || resource.mime;
        resource.filename = resource.filename && resource.filename !== "resource" ? resource.filename : merged.filename;
        resource.supportsRange = merged.supportsRange || resource.supportsRange;
      }
    }
    trimBucket(resource.tabId);
    scheduleBridgeStatePersist();
  }

  function findResourceByUrl(url: string, tabId?: number): CapturedResource | null {
    const exactResourceIdSuffix = `:${urlWithoutHash(url, true)}`;
    const normalizedTarget = comparableUrl(url);
    let matched: CapturedResource | null = null;
    const resources = tabId == null ? resourcesById.values() : (resourceCache.get(tabId)?.values() ?? []);
    for (const resource of resources) {
      const exactMatch = resource.id.endsWith(exactResourceIdSuffix);
      const normalizedMatch = normalizedTarget && comparableUrl(resource.url) === normalizedTarget;
      const pathMatch = sameOriginPath(resource.url, url);
      if (!exactMatch && !normalizedMatch && !pathMatch) {
        continue;
      }
      if (matched == null || resource.capturedAt > matched.capturedAt) {
        matched = resource;
      }
    }
    return matched;
  }

  function findTwitterMasterByMediaId(mediaId: string, tabId?: number): CapturedResource | null {
    const normalizedMediaId = mediaId.trim();
    if (!normalizedMediaId) {
      return null;
    }

    let matched: CapturedResource | null = null;
    const resources = tabId == null ? resourcesById.values() : (resourceCache.get(tabId)?.values() ?? []);
    for (const resource of resources) {
      if (mediaIdentifierFromUrl(resource.url) !== normalizedMediaId) {
        continue;
      }
      if (!isTwitterMasterPlaylistUrl(resource.url)) {
        continue;
      }
      if (matched == null || resource.capturedAt > matched.capturedAt) {
        matched = resource;
      }
    }
    return matched;
  }

  function markResourceSent(resourceId: string) {
    const resource = resourcesById.get(resourceId);
    if (!resource) {
      return;
    }
    resource.sentToDesktopAt = Date.now();
    scheduleBridgeStatePersist();
  }

  function rememberHeaderSnapshot(
    url: string,
    headers: Record<string, string>,
    tabId: number | null,
    supportsRange: boolean,
  ) {
    const existing = headerSnapshotsByUrl.get(url);
    const mergedHeaders = {
      ...(existing?.headers ?? {}),
      ...headers,
    };
    const mergedSupportsRange = supportsRange || Boolean(existing?.supportsRange);
    if (Object.keys(mergedHeaders).length === 0 && !mergedSupportsRange) {
      return;
    }
    headerSnapshotsByUrl.set(url, {
      url,
      headers: mergedHeaders,
      capturedAt: Date.now(),
      tabId: tabId ?? existing?.tabId ?? null,
      supportsRange: mergedSupportsRange,
    });
    pruneHeaderSnapshots();
    scheduleBridgeStatePersist();
    notifyHeaderSnapshotCached(url, tabId ?? existing?.tabId ?? null);
  }

  function resolveHeaderSnapshot(url: string): HeaderSnapshot | null {
    pruneHeaderSnapshots();
    return headerSnapshotsByUrl.get(url) ?? null;
  }

  function resolveHeaderSnapshotMatch(url: string, tabId?: number | null): HeaderSnapshotMatch | null {
    pruneHeaderSnapshots();
    const exact = headerSnapshotsByUrl.get(url);
    if (exact) {
      return { snapshot: exact, strategy: "exact" };
    }

    const normalizedTarget = comparableUrl(url);
    let normalizedMatch: HeaderSnapshot | null = null;
    let pathMatch: HeaderSnapshot | null = null;
    for (const snapshot of headerSnapshotsByUrl.values()) {
      if (tabId != null && snapshot.tabId != null && snapshot.tabId !== tabId) {
        continue;
      }
      if (normalizedTarget && comparableUrl(snapshot.url) === normalizedTarget) {
        if (normalizedMatch == null || snapshot.capturedAt > normalizedMatch.capturedAt) {
          normalizedMatch = snapshot;
        }
        continue;
      }
      if (sameOriginPath(snapshot.url, url)) {
        if (pathMatch == null || snapshot.capturedAt > pathMatch.capturedAt) {
          pathMatch = snapshot;
        }
      }
    }

    if (normalizedMatch) {
      return { snapshot: normalizedMatch, strategy: "normalized" };
    }
    if (pathMatch) {
      return { snapshot: pathMatch, strategy: "same_path" };
    }
    return null;
  }

  function resolveHeaderSnapshotMatchAny(urls: string[], tabId?: number | null): HeaderSnapshotMatch | null {
    let newest: HeaderSnapshotMatch | null = null;
    for (const url of urls) {
      if (!url) {
        continue;
      }
      const match = resolveHeaderSnapshotMatch(url, tabId);
      if (!match) {
        continue;
      }
      if (!newest || match.snapshot.capturedAt > newest.snapshot.capturedAt) {
        newest = match;
      }
    }
    return newest;
  }

  function notifyHeaderSnapshotCached(url: string, tabId: number | null): void {
    for (const awaiter of [...headerSnapshotAwaiters]) {
      if (awaiter.tabId != null && tabId != null && awaiter.tabId !== tabId) {
        continue;
      }
      const match = resolveHeaderSnapshotMatchAny(awaiter.urls, awaiter.tabId);
      if (!match) {
        continue;
      }
      headerSnapshotAwaiters.delete(awaiter);
      clearTimeout(awaiter.timer);
      awaiter.resolve(match);
    }
  }

  function waitForHeaderSnapshotMatch(
    urls: string[],
    tabId: number | null,
    timeoutMs = 1500,
  ): Promise<HeaderSnapshotMatch | null> {
    const uniqueUrls = [...new Set(urls.filter(Boolean))];
    const immediate = resolveHeaderSnapshotMatchAny(uniqueUrls, tabId);
    if (immediate || uniqueUrls.length === 0) {
      return Promise.resolve(immediate);
    }

    return new Promise((resolve) => {
      const awaiter: HeaderSnapshotAwaiter = {
        urls: uniqueUrls,
        tabId,
        resolve,
        timer: self.setTimeout(() => {
          headerSnapshotAwaiters.delete(awaiter);
          resolve(resolveHeaderSnapshotMatchAny(uniqueUrls, tabId));
        }, timeoutMs),
      };
      headerSnapshotAwaiters.add(awaiter);
    });
  }

  function otherResourcesForTab(activeTabId: number | null): CapturedResource[] {
    const result: CapturedResource[] = [];
    for (const [tabId, bucket] of resourceCache.entries()) {
      if (activeTabId != null && tabId === activeTabId) {
        continue;
      }
      result.push(...bucket.values());
    }
    return sortResources(result);
  }

  function mergeOutputTitle(resources: CapturedResource[]): string {
    const pageTitle = (resources[0]?.pageTitle ?? "").trim();
    if (pageTitle) {
      return pageTitle;
    }

    const firstFileName = basenameOf(resources[0]?.filename || filenameFromUrl(resources[0]?.url || ""));
    if (firstFileName) {
      const extension = fileExtension(firstFileName);
      return extension ? firstFileName.slice(0, -(extension.length + 1)) : firstFileName;
    }

    return "merged-media";
  }

  async function downloadResourceViaBrowser(resource: CapturedResource): Promise<void> {
    const filename = filenameForDesktop(resource);

    return new Promise((resolve, reject) => {
      chrome.downloads.download(
        {
          url: resource.url,
          filename,
        },
        (downloadId) => {
          const lastError = chrome.runtime.lastError;
          if (lastError) {
            reject(new Error(lastError.message));
            return;
          }
          if (typeof downloadId !== "number") {
            reject(new Error("浏览器未返回下载任务"));
            return;
          }
          resolve();
        },
      );
    });
  }

  async function loadPersistentState() {
    const bridgeState = await loadFromBridgeStorage<{
      [BRIDGE_RESOURCE_CACHE_KEY]: Record<string, CapturedResource[]>;
      [BRIDGE_HEADER_SNAPSHOTS_KEY]: HeaderSnapshot[];
      [BRIDGE_LAST_ACTIVE_TAB_KEY]: number;
    }>({
      [BRIDGE_RESOURCE_CACHE_KEY]: {},
      [BRIDGE_HEADER_SNAPSHOTS_KEY]: [],
      [BRIDGE_LAST_ACTIVE_TAB_KEY]: 0,
    });

    resourceCache.clear();
    resourcesById.clear();
    for (const [tabIdText, resources] of Object.entries(bridgeState[BRIDGE_RESOURCE_CACHE_KEY] ?? {})) {
      const tabId = Number(tabIdText);
      if (!Number.isInteger(tabId) || tabId <= 0 || !Array.isArray(resources)) {
        continue;
      }
      const bucket = new Map<string, CapturedResource>();
      for (const resource of sortResources(resources.map(resourceFromStorage)).slice(0, RESOURCE_LIMIT)) {
        bucket.set(resource.id, resource);
        resourcesById.set(resource.id, resource);
      }
      resourceCache.set(tabId, bucket);
    }

    headerSnapshotsByUrl.clear();
    for (const snapshot of bridgeState[BRIDGE_HEADER_SNAPSHOTS_KEY] ?? []) {
      const normalized = headerSnapshotFromStorage(snapshot);
      if (!normalized.url) {
        continue;
      }
      headerSnapshotsByUrl.set(normalized.url, normalized);
    }
    pruneHeaderSnapshots();

    lastActiveTabId = Number(bridgeState[BRIDGE_LAST_ACTIVE_TAB_KEY] ?? 0) || null;
    restoredFromStorage = true;
  }

  function cacheResourceFor(
    url: string,
    tabId: number,
    tab: chrome.tabs.Tab | null,
    pageUrl: string,
    parts: {
      filename: string;
      mime: string;
      size: number;
      supportsRange: boolean;
      extraHeaders?: Record<string, string>;
    },
  ) {
    const headerSnapshot = resolveHeaderSnapshotMatch(url, tabId)?.snapshot ?? null;
    const headers = {
      ...(headerSnapshot?.headers ?? {}),
      ...(parts.extraHeaders ?? {}),
    };
    const referer = headers.referer || pageUrl || tab?.url || "";
    if (referer) { headers.referer = referer; }

    cacheResource({
      id: `${tabId}:${urlWithoutHash(url, true)}`,
      tabId,
      url,
      pageTitle: tab?.title ?? "",
      pageUrl: pageUrl || tab?.url || "",
      filename: parts.filename,
      mime: parts.mime,
      size: parts.size,
      supportsRange: parts.supportsRange || Boolean(headerSnapshot?.supportsRange),
      referer,
      requestHeaders: headers,
      capturedAt: Date.now(),
    });
  }

  // Cross-origin opaque requests get the literal string "null" here.
  function initiatorOf(details: { initiator?: string }): string {
    return details.initiator && details.initiator !== "null" ? details.initiator : "";
  }

  async function capturePageResource(sender: chrome.runtime.MessageSender, payload: CapturePayload) {
    const tabId = await resolveBridgeResourceTabId(sender, payload.href);
    if (!tabId || !/^(https?:|blob:)/i.test(payload.url)) {
      return;
    }

    const tab = await findTab(tabId);
    cacheResourceFor(payload.url, tabId, tab, payload.href ?? tab?.url ?? "", {
      filename: resolveBridgeFilename(payload),
      mime: payload.mime?.toLowerCase() || mimeFromUrl(payload.url),
      size: 0,
      supportsRange: false,
      extraHeaders: selectAllowedHeaders(payload.requestHeaders),
    });
  }

  async function captureNetworkResource(details: chrome.webRequest.OnResponseStartedDetails) {
    const meta = responseMeta(details.responseHeaders);
    meta.mime = mimeFromUrl(details.url) || meta.mime;
    const responseSupportsRange = meta.supportsRange || details.statusCode === 206;
    if (responseSupportsRange && isCapturableUrl(details.url)) {
      rememberHeaderSnapshot(details.url, {}, details.tabId > 0 ? details.tabId : lastActiveTabId, true);
    }

    if (!shouldCaptureNetworkResource(details, meta)) {
      return;
    }

    const tabId = await resolveNetworkResourceTabId(details);
    if (!tabId) {
      return;
    }

    const tab = await findTab(tabId);
    cacheResourceFor(details.url, tabId, tab, initiatorOf(details), {
      filename: meta.filename || basenameOf(filenameFromUrl(details.url)) || "resource",
      mime: meta.mime,
      size: meta.size,
      supportsRange: responseSupportsRange,
    });
  }

  async function captureRequestResource(details: chrome.webRequest.OnSendHeadersDetails) {
    if (!shouldCaptureRequestResource(details)) {
      return;
    }

    const tabId = details.tabId > 0 ? details.tabId : (lastActiveTabId ?? await resolveActiveTabId());
    if (!tabId) {
      return;
    }

    const tab = await findTab(tabId);
    cacheResourceFor(details.url, tabId, tab, initiatorOf(details), {
      filename: basenameOf(filenameFromUrl(details.url)) || "resource",
      mime: mimeFromUrl(details.url),
      size: 0,
      supportsRange: false,
    });
  }

  async function handoffBrowserDownload(downloadItem: chrome.downloads.DownloadItem) {
    if (!isAbsoluteDownloadPath(downloadItem.filename)) {
      throw new Error("浏览器尚未确认完整保存路径，不接管此次下载");
    }
    const traceId = traceIdFor("browser_download");
    const finalUrl = downloadItem.finalUrl || downloadItem.url;
    const matchedResource = findResourceByUrl(finalUrl) ?? findResourceByUrl(downloadItem.url);
    const resolvedFilename =
      basenameOf(downloadItem.filename)
      || basenameOf(matchedResource?.filename ?? "")
      || basenameOf(filenameFromUrl(finalUrl))
      || "resource";

    const matchedTabId = matchedResource?.tabId ?? null;
    const headerMatch = await waitForHeaderSnapshotMatch([finalUrl, downloadItem.url], matchedTabId);
    const headerSnapshot = headerMatch?.snapshot ?? null;
    const headers = { ...(headerSnapshot?.headers ?? {}) };
    if (downloadItem.referrer && !headers.referer) {
      headers.referer = downloadItem.referrer;
    }
    const pageUrl = matchedResource?.pageUrl || downloadItem.referrer || finalUrl;
    const browserCookies = await encryptedCookiesForUrl(pageUrl);

    try {
      const result = await options.createTask({
        request_id: requestIdFor("download", `${downloadItem.id}:${finalUrl}`),
        source: "download",
        title: resolvedFilename,
        payload: {
          url: finalUrl,
          headers,
          filename: resolvedFilename,
          save_path: downloadItem.filename,
          defer_start: options.startBrowserTask ? true : undefined,
          mime: downloadItem.mime || matchedResource?.mime || mimeFromUrl(finalUrl) || undefined,
          size:
            typeof downloadItem.totalBytes === "number" && downloadItem.totalBytes > 0
              ? downloadItem.totalBytes
              : matchedResource?.size ?? 0,
          supports_range: Boolean(
            matchedResource?.supportsRange
            || headerSnapshot?.supportsRange
            || downloadItem.canResume === true,
          ),
          encrypted_cookies: browserCookies.encrypted_cookies,
          cookie_iv: browserCookies.cookie_iv,
        },
        client_context: clientContext(matchedResource?.tabId ?? null, pageUrl, traceId),
      });
      options.recordTelemetry?.({
        event_type: "client.dispatch_context",
        trace_id: traceId,
        request_id: requestIdFor("download", `${downloadItem.id}:${finalUrl}`),
        surface: "background",
        action: "browser_download_headers",
        source: "download",
        status: headerSnapshot ? "matched" : "missing",
        url_host: domainFromUrl(finalUrl),
        details: {
          matchStrategy: headerMatch?.strategy ?? "none",
          headerNames: Object.keys(headers).sort(),
          hasMatchedResource: Boolean(matchedResource),
          supportsRange: Boolean(headerSnapshot?.supportsRange),
        },
      });
      if (!result.ok) {
        throw new Error(result.message || "browser download handoff was rejected by daemon");
      }
      if (options.startBrowserTask) {
        if (!result.taskId) throw new Error("backend accepted a browser task without an ID");
        return () => options.startBrowserTask!(result.taskId!);
      }
      // Task successfully created, no need to open action popup for seamless background download
    } catch (error) {
      // Browser download was already intercepted — the user already lost it from the
      // browser's download tray, so a desktop handoff failure here is unrecoverable anyway.
      throw error;
    }
  }

  async function sendHttpResourceToDesktop(resource: CapturedResource, traceId?: string): Promise<DesktopRequestResult> {
    const mediaId = mediaIdentifierFromUrl(resource.url);
    const title = mediaId ? titleForPageMediaResource(resource) : filenameForDesktop(resource);
    const filename = mediaId ? filenameForPageMediaResource(resource) : title;

    const browserCookies = await encryptedCookiesForUrl(resource.pageUrl || resource.url);

    try {
      const result = await options.createTask({
        request_id: requestIdFor("resource", resource.id || resource.url),
        source: "resource",
        title,
        payload: {
          url: resource.url,
          headers: resource.requestHeaders,
          filename,
          mime: resource.mime || undefined,
          size: resource.size,
          supports_range: resource.supportsRange,
          encrypted_cookies: browserCookies.encrypted_cookies,
          cookie_iv: browserCookies.cookie_iv,
        },
        client_context: clientContext(resource.tabId, resource.pageUrl, traceId),
      });

      if (result.ok) {
        markResourceSent(resource.id);
        return {
          ...result,
          message: result.message || "资源已发送到 Nexus Downloader",
        };
      }
      return result;
    } catch (error) {
      return {
        ok: false,
        traceId,
        message: error instanceof Error ? error.message : "发送资源失败",
      };
    }
  }

  async function sendResource(resourceId: string, traceId = traceIdFor("popup_resource")): Promise<DesktopRequestResult> {
    const resource = resourcesById.get(resourceId) ?? null;
    if (!resource) {
      recordUiFailure(traceId, "send_resource", "资源不存在", { resourceId });
      return { ok: false, traceId, message: "资源不存在" };
    }

    if (resource.url.startsWith("blob:")) {
      try {
        await downloadResourceViaBrowser(resource);
        markResourceSent(resource.id);
        return { ok: true, traceId, message: "资源已交给浏览器下载" };
      } catch (error) {
        recordUiFailure(traceId, "send_resource", error instanceof Error ? error.message : "发送资源失败", { resourceId });
        return { ok: false, traceId, message: error instanceof Error ? error.message : "发送资源失败" };
      }
    }

    return sendHttpResourceToDesktop(resource, traceId);
  }

  async function sendPageMediaResourceToDesktop(resource: CapturedResource, traceId?: string): Promise<DesktopRequestResult> {
    const resultTitle = titleForPageMediaResource(resource);
    const browserCookies = await encryptedCookiesForUrl(resource.pageUrl || resource.url);
    try {
      const result = await options.createTask({
        request_id: uniqueRequestIdFor("page_media", resource.id || resource.url),
        source: "page_media",
        title: resultTitle,
        payload: {
          url: resource.url,
          headers: resource.requestHeaders,
          filename: filenameForPageMediaResource(resource),
          mime: resource.mime || undefined,
          size: resource.size,
          supports_range: resource.supportsRange,
          encrypted_cookies: browserCookies.encrypted_cookies,
          cookie_iv: browserCookies.cookie_iv,
        },
        client_context: clientContext(resource.tabId, resource.pageUrl, traceId),
      });

      if (result.ok) {
        markResourceSent(resource.id);
        return {
          ...result,
          message: result.message || "媒体已发送到 Nexus Downloader",
        };
      }
      return result;
    } catch (error) {
      return {
        ok: false,
        traceId,
        message: error instanceof Error ? error.message : "发送媒体失败",
      };
    }
  }

  // Twitter / Bilibili CDNs 403 without Referer; cat-catch's addMedia path doesn't carry one.
  function ensureReferer(resource: CapturedResource, fallback: string): void {
    if (!fallback || resource.requestHeaders.referer || resource.referer) { return; }
    resource.requestHeaders = { ...resource.requestHeaders, referer: fallback };
    resource.referer = fallback;
  }

  // Strategy already proved this URL belongs to the active video, so we must dispatch
  // something — desktop range-probes whatever metadata we can't fill in. The synthesized
  // row goes into resourceCache so markResourceSent can find it and a later webRequest
  // event can merge real size/headers into it.
  async function resolveResourceForUrl(url: string, tabId: number, fallbackTitle: string, fallbackPageUrl: string): Promise<CapturedResource> {
    const normalizedFallbackTitle = cleanFilename(fallbackTitle);
    const id = `${tabId}:${urlWithoutHash(url, true)}`;

    const direct = resourceCache.get(tabId)?.get(id);
    if (direct) {
      ensureReferer(direct, fallbackPageUrl);
      return {
        ...direct,
        pageTitle: normalizedFallbackTitle && !isGenericXPageTitle(normalizedFallbackTitle)
          ? normalizedFallbackTitle
          : direct.pageTitle,
        pageUrl: fallbackPageUrl || direct.pageUrl,
      };
    }

    const waited = await awaitResourceCached(id, 1500);
    if (waited) {
      ensureReferer(waited, fallbackPageUrl);
      return {
        ...waited,
        pageTitle: normalizedFallbackTitle && !isGenericXPageTitle(normalizedFallbackTitle)
          ? normalizedFallbackTitle
          : waited.pageTitle,
        pageUrl: fallbackPageUrl || waited.pageUrl,
      };
    }

    const snapshot = resolveHeaderSnapshotMatch(url, tabId)?.snapshot ?? null;
    const headers: Record<string, string> = { ...(snapshot?.headers ?? {}) };
    const referer = headers.referer || fallbackPageUrl;
    if (referer) { headers.referer = referer; }
    const synthesized: CapturedResource = {
      id,
      tabId,
      url,
      pageTitle: normalizedFallbackTitle || fallbackTitle,
      pageUrl: fallbackPageUrl,
      filename: basenameOf(filenameFromUrl(url)) || "resource",
      mime: mimeFromUrl(url),
      size: 0,
      supportsRange: Boolean(snapshot?.supportsRange),
      referer,
      requestHeaders: headers,
      capturedAt: Date.now(),
    };
    cacheResource(synthesized);
    console.warn(
      "[GD3 Bridge] resolveResourceForUrl synthesized fallback (no webRequest row within 1500ms)",
      { url: synthesized.url, hasSnapshot: Boolean(snapshot), hasReferer: Boolean(referer) },
    );
    return synthesized;
  }

  async function resolveTwitterMasterByMediaId(
    mediaId: string,
    tabId: number,
    fallbackTitle: string,
    fallbackPageUrl: string,
  ): Promise<CapturedResource | null> {
    const direct = findTwitterMasterByMediaId(mediaId, tabId);
    if (direct) {
      ensureReferer(direct, fallbackPageUrl);
      return {
        ...direct,
        pageTitle: cleanFilename(fallbackTitle) || direct.pageTitle,
        pageUrl: fallbackPageUrl || direct.pageUrl,
      };
    }

    const waited = await awaitResourceMatch(
      (resource) =>
        resource.tabId === tabId
        && mediaIdentifierFromUrl(resource.url) === mediaId
        && isTwitterMasterPlaylistUrl(resource.url),
      4000,
    );
    if (waited) {
      ensureReferer(waited, fallbackPageUrl);
      return {
        ...waited,
        pageTitle: cleanFilename(fallbackTitle) || waited.pageTitle,
        pageUrl: fallbackPageUrl || waited.pageUrl,
      };
    }

    return null;
  }

  async function downloadPageMedia(sender: chrome.runtime.MessageSender, payload: ClickPayload): Promise<DesktopRequestResult> {
    const traceId = payload.traceId || traceIdFor("overlay");
    const tabId = await resolveBridgeResourceTabId(sender, payload.href);
    if (!tabId) {
      recordUiFailure(traceId, "page_download_media", "当前没有可操作的标签页", { href: payload.href });
      return { ok: false, traceId, message: "当前没有可操作的标签页" };
    }

    const selection = payload.selection;
    if (!selection || typeof selection !== "object") {
      recordUiFailure(traceId, "page_download_media", "无效的下载请求");
      return { ok: false, traceId, message: "无效的下载请求" };
    }
    const fallbackPageUrl = payload.href || "";

    if (selection.kind === "stream_by_media_id") {
      const mediaId = String(selection.mediaId ?? "").trim();
      if (!mediaId) {
        recordUiFailure(traceId, "page_download_media", "无效的 X 媒体标识", { selectionKind: selection.kind });
        return { ok: false, traceId, message: "无效的 X 媒体标识" };
      }
      const resource = await resolveTwitterMasterByMediaId(mediaId, tabId, payload.title, fallbackPageUrl);
      if (!resource) {
        recordUiFailure(traceId, "page_download_media", "未捕获到当前 X 视频主播放清单", {
          selectionKind: selection.kind,
          mediaId,
        });
        return { ok: false, traceId, message: "未捕获到当前 X 视频主播放清单，请让视频播放一小段后重试" };
      }
      const result = await sendPageMediaResourceToDesktop(resource, traceId);
      return result;
    }

    if (selection.kind === "single" || selection.kind === "stream") {
      if (!selection.url) {
        recordUiFailure(traceId, "page_download_media", "无效的下载请求", { selectionKind: selection.kind });
        return { ok: false, traceId, message: "无效的下载请求" };
      }
      const resource = await resolveResourceForUrl(selection.url, tabId, payload.title, fallbackPageUrl);
      const result = await sendPageMediaResourceToDesktop(resource, traceId);
      return result;
    }

    if (selection.kind === "merge") {
      if (!selection.video || !selection.audio) {
        recordUiFailure(traceId, "page_download_media", "无效的合并请求");
        return { ok: false, traceId, message: "无效的合并请求" };
      }
      const [video, audio] = await Promise.all([
        resolveResourceForUrl(selection.video, tabId, payload.title, fallbackPageUrl),
        resolveResourceForUrl(selection.audio, tabId, payload.title, fallbackPageUrl),
      ]);
      const result = await dispatchMergeResources([video, audio], traceId);
      return result;
    }

    recordUiFailure(traceId, "page_download_media", "未知的下载请求类型");
    return { ok: false, traceId, message: "未知的下载请求类型" };
  }

  async function dispatchMergeResources(resources: CapturedResource[], traceId?: string): Promise<DesktopRequestResult> {
    if (resources.length !== 2) {
      recordUiFailure(traceId, "merge_resources", "在线合并暂时只支持选中 2 个资源", { resourceCount: resources.length });
      return { ok: false, traceId, message: "在线合并暂时只支持选中 2 个资源" };
    }
    const ordered = sortResourcesForOnlineMerge(resources);
    try {
      const result = await options.createTask({
        request_id: requestIdFor("resource_merge", ordered.map((resource) => resource.id || resource.url).join("|")),
        source: "resource_merge",
        title: mergeOutputTitle(ordered),
        payload: {
          url: ordered[0]?.url ?? "",
          resources: ordered.map(capturedResourcePayload),
        },
        client_context: clientContext(ordered[0]?.tabId ?? null, ordered[0]?.pageUrl, traceId),
      });
      if (result.ok) {
        ordered.forEach((resource) => markResourceSent(resource.id));
        return { ...result, message: result.message || "在线合并任务已发送到 Nexus Downloader" };
      }
      return result;
    } catch (error) {
      recordUiFailure(traceId, "merge_resources", error instanceof Error ? error.message : "在线合并失败");
      return { ok: false, traceId, message: error instanceof Error ? error.message : "在线合并失败" };
    }
  }

  async function mergeResources(resourceIds: string[], traceId = traceIdFor("popup_merge")): Promise<DesktopRequestResult> {
    const ids = [...new Set(resourceIds.filter(Boolean))];
    const resources = ids
      .map((resourceId) => resourcesById.get(resourceId) ?? null)
      .filter((resource): resource is CapturedResource => resource != null);

    if (!canUseOnlineMergeSelection(resources)) {
      recordUiFailure(traceId, "merge_resources", "当前选中的资源不符合在线合并条件", { requestedCount: ids.length, resolvedCount: resources.length });
      return { ok: false, traceId, message: "当前选中的资源不符合在线合并条件" };
    }

    return dispatchMergeResources(resources, traceId);
  }

  function buildPopupStateData(resolvedTabId: number | null, activeTab: chrome.tabs.Tab | null) {
    const canCaptureCurrentTab = Boolean(activeTab?.url && isCapturableUrl(activeTab.url));
    let resourceState: ResourceCollectionState = "ready";
    let resourceStateMessage = "等待 cat-catch 捕获资源";
    if (!restoredFromStorage) {
      resourceState = "restoring";
      resourceStateMessage = "正在恢复 cat-catch 已捕获的资源";
    } else if (!canCaptureCurrentTab) {
      resourceState = "unavailable";
      resourceStateMessage = "当前标签页不支持 cat-catch 资源桥接";
    }

    return {
      resourceState,
      resourceStateMessage,
      currentResources: resolvedTabId == null ? [] : sortResources(resourceCache.get(resolvedTabId)?.values() ?? []),
      otherResources: otherResourcesForTab(resolvedTabId),
      activePageDomain: domainFromUrl(activeTab?.url ?? ""),
    };
  }

  function onTabRemoved(tabId: number) {
    clearResourcesForTab(tabId);
    clearHeaderSnapshotsForTab(tabId);
    if (lastActiveTabId === tabId) {
      void setLastActiveTab(null);
    }
  }

  function onNavigationCommitted(details: chrome.webNavigation.WebNavigationTransitionCallbackDetails) {
    if (details.tabId <= 0 || details.frameId !== 0) {
      return;
    }
    clearResourcesForTab(details.tabId);
    clearHeaderSnapshotsForTab(details.tabId);
  }

  function onRequestHeaders(details: chrome.webRequest.OnSendHeadersDetails) {
    const headers = selectAllowedHeaders(Object.fromEntries((details.requestHeaders ?? []).map((header) => [header.name ?? "", header.value ?? ""])));
    const supportsRange = (details.requestHeaders ?? []).some((header) => header.name?.toLowerCase() === "range" && String(header.value ?? "").toLowerCase().startsWith("bytes="));
    rememberHeaderSnapshot(details.url, headers, details.tabId > 0 ? details.tabId : lastActiveTabId, supportsRange);
  }

  return {
    buildPopupStateData,
    captureNetworkResource,
    capturePageResource,
    captureRequestResource,
    downloadPageMedia,
    handoffBrowserDownload,
    onNavigationCommitted,
    onRequestHeaders,
    onTabRemoved,
    loadPersistentState,
    refreshActiveTabFromBrowser,
    resolveActiveTabId,
    mergeResources,
    sendResource,
    setLastActiveTab,
  };
}
