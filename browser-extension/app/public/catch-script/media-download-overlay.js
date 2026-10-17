(function installGhostDownloaderMediaButton() {
    if (window.GhostDownloaderMediaButton?.installed) { return; }
    if (!globalThis.chrome?.runtime?.sendMessage) { return; }

    const host = document.createElement("div");
    const root = host.attachShadow({ mode: "open" });
    let resetTimer = 0;
    let updateQueued = false;
    let enabled = false;
    let hostHovered = false;
    let fullscreenRevealTimer = 0;

    host.id = "ghostDownloaderMediaDownload";
    Object.assign(host.style, {
        display: "none",
        left: "0",
        position: "fixed",
        top: "0",
        zIndex: "1000000000",
    });

    root.innerHTML = `
        <style>
            :host {
                font: 13px/18px "Segoe UI", "Microsoft YaHei", Arial, sans-serif;
                transition: opacity 0.3s ease;
            }
            :host(.fullscreen-auto-hide) {
                opacity: 0;
                pointer-events: none;
            }
            :host(.fullscreen-auto-hide:hover),
            :host(.fullscreen-auto-hide.fullscreen-hover),
            :host(.fullscreen-auto-hide.fullscreen-busy) {
                opacity: 1;
                pointer-events: auto;
            }
            button {
                align-items: center;
                background: rgba(255, 255, 255, 0.96);
                border: 1px solid #d1d1d1;
                border-radius: 4px;
                box-shadow: 0 8px 20px rgba(0, 0, 0, 0.18), 0 1px 4px rgba(0, 0, 0, 0.12);
                color: #242424;
                cursor: pointer;
                display: inline-flex;
                font: inherit;
                font-weight: 600;
                gap: 6px;
                min-height: 32px;
                padding: 6px 10px;
                white-space: nowrap;
            }
            button:hover {
                background: #f5f5f5;
            }
            button:active {
                background: #e0e0e0;
            }
            button:disabled {
                cursor: default;
                opacity: 0.78;
            }
            .icon {
                width: 16px;
                height: 16px;
                flex: none;
            }
            .status {
                color: #0f6cbd;
            }
            .error {
                color: #b10e1c;
            }
        </style>
        <button type="button" title="发送当前媒体到 Nexus Downloader">
            <svg class="icon" viewBox="0 0 20 20" aria-hidden="true">
                <path fill="currentColor" d="M10 2.5a.75.75 0 0 1 .75.75v7.69l2.72-2.72a.75.75 0 1 1 1.06 1.06l-4 4a.75.75 0 0 1-1.06 0l-4-4a.75.75 0 0 1 1.06-1.06l2.72 2.72V3.25A.75.75 0 0 1 10 2.5Zm-5.25 11a.75.75 0 0 1 .75.75v1.25h9v-1.25a.75.75 0 0 1 1.5 0v2a.75.75 0 0 1-.75.75H4.75A.75.75 0 0 1 4 16.25v-2a.75.75 0 0 1 .75-.75Z"/>
            </svg>
            <span class="label">下载此媒体</span>
            <span class="status"></span>
        </button>
    `;

    const button = root.querySelector("button");
    const label = root.querySelector(".label");
    const status = root.querySelector(".status");
    let currentMedia = null;
    let mediaCounter = 0;
    const stateByMedia = new WeakMap();
    const stateByTaskId = new Map();
    const TERMINAL_STATE_TTL_MS = 30000;
    const overlayStateCore = globalThis.GD3OverlayMediaState || null;

    function createTraceId() {
        const random = typeof crypto?.randomUUID === "function"
            ? crypto.randomUUID()
            : `${Date.now().toString(36)}_${Math.random().toString(36).slice(2)}`;
        return `trace_overlay_${random}`.replace(/[^a-zA-Z0-9_-]/g, "_").slice(0, 96);
    }

    function urlHost(value) {
        try { return new URL(value).host; } catch { return value ? "invalid" : undefined; }
    }

    function urlPathKind(value) {
        try {
            const path = new URL(value).pathname.toLowerCase();
            if (path.endsWith(".m3u8")) { return "m3u8"; }
            if (path.endsWith(".mp4")) { return "mp4"; }
            if (path.endsWith(".m4s")) { return "m4s"; }
            if (path.endsWith(".mpd")) { return "mpd"; }
            return "other";
        } catch {
            return value ? "invalid" : undefined;
        }
    }

    function cleanTitleText(value) {
        return String(value || "")
            .replace(/\s+/g, " ")
            .replace(/[<>:"/\\|?*\x00-\x1f]+/g, " ")
            .trim();
    }

    function isXLikePage() {
        return /(^|\.)x\.com$/i.test(location.hostname) || /(^|\.)twitter\.com$/i.test(location.hostname);
    }

    function isNoiseTitleLine(value) {
        return !value
            || value === "·"
            || value === "Follow"
            || value === "Following"
            || value === "关注"
            || value === "正在关注"
            || /^\d+[smhd]$/i.test(value)
            || /^\d+[秒分钟小时天]/.test(value);
    }

    function extractXArticleTitle(media) {
        if (!isXLikePage()) { return ""; }
        const article = media?.closest?.("article");
        if (!article) { return ""; }

        const lines = String(article.innerText || "")
            .split(/\n+/)
            .map(cleanTitleText)
            .filter((line) => !isNoiseTitleLine(line));
        const handleLineIndex = lines.findIndex((line) => /^@[A-Za-z0-9_]{1,15}$/.test(line));
        const handle = handleLineIndex >= 0 ? lines[handleLineIndex].slice(1) : "";
        let displayName = "";

        if (handleLineIndex > 0) {
            for (let index = handleLineIndex - 1; index >= 0; index -= 1) {
                const line = lines[index];
                if (!line || line.startsWith("@") || isNoiseTitleLine(line)) { continue; }
                displayName = line;
                break;
            }
        }

        if (!displayName) {
            for (const link of article.querySelectorAll("a[href]")) {
                const text = cleanTitleText(link.textContent);
                if (!text || text.startsWith("@") || isNoiseTitleLine(text)) { continue; }
                try {
                    const href = new URL(link.href);
                    if (!/(^|\.)x\.com$/i.test(href.hostname) && !/(^|\.)twitter\.com$/i.test(href.hostname)) { continue; }
                    const parts = href.pathname.split("/").filter(Boolean);
                    if (parts.length !== 1 || ["home", "i", "search", "explore"].includes(parts[0])) { continue; }
                    displayName = text;
                    break;
                } catch {
                    // Ignore malformed profile links.
                }
            }
        }

        if (displayName && handle) { return `${displayName} (@${handle}) X`; }
        if (displayName) { return `${displayName} X`; }
        if (handle) { return `@${handle} X`; }
        return "";
    }

    function titleForMedia(media) {
        return extractXArticleTitle(media) || document.title;
    }

    function articleStatusKey(media) {
        const article = media?.closest?.("article");
        if (!article) { return ""; }
        for (const link of article.querySelectorAll("a[href]")) {
            try {
                const href = new URL(link.href, location.href);
                const parts = href.pathname.split("/").filter(Boolean);
                const statusIndex = parts.findIndex((part) => part === "status");
                if (statusIndex >= 0 && parts[statusIndex + 1]) {
                    return `${href.host}/${parts.slice(0, statusIndex + 2).join("/")}`;
                }
            } catch {
                // Ignore malformed feed links.
            }
        }
        return "";
    }

    function mediaIdentity(media) {
        const statusKey = articleStatusKey(media);
        if (statusKey) {
            return [
                titleForMedia(media),
                statusKey,
                media?.poster || "",
            ].join("|");
        }
        return [
            titleForMedia(media),
            media?.poster || "",
            media?.currentSrc || media?.src || "",
        ].join("|");
    }

    function createMediaState(media) {
        mediaCounter += 1;
        const input = {
            mediaKey: `media_${mediaCounter}`,
            mediaTitle: titleForMedia(media),
            identity: mediaIdentity(media),
        };
        if (overlayStateCore?.createOverlayMediaState) {
            return overlayStateCore.createOverlayMediaState(input);
        }
        return {
            ...input,
            traceId: "",
            taskId: "",
            requestId: "",
            phase: "idle",
            message: "",
            speed: 0,
            progress: 0,
            failed: false,
            selectionKind: "",
            lastUiStateSignature: "",
            lastDownloadUiStateSignature: "",
            terminalTimer: 0,
            missingTaskTimer: 0,
        };
    }

    function stateForMedia(media) {
        if (!media) { return null; }
        const identity = mediaIdentity(media);
        let state = stateByMedia.get(media);
        if (!state || state.identity !== identity) {
            state = createMediaState(media);
            stateByMedia.set(media, state);
        }
        return state;
    }

    function isRenderedState(state) {
        return Boolean(state && currentMedia && stateByMedia.get(currentMedia) === state);
    }

    function isBusyState(state) {
        return overlayStateCore?.isBusyOverlayState
            ? overlayStateCore.isBusyOverlayState(state)
            : Boolean(state && ["resolving", "waiting", "dispatching", "queued", "starting", "parsing", "downloading", "merging"].includes(state.phase));
    }

    function clearStateTimers(state) {
        clearTimeout(state.missingTaskTimer);
        clearTimeout(state.terminalTimer);
        state.missingTaskTimer = 0;
        state.terminalTimer = 0;
    }

    function resetMediaState(media, state = stateForMedia(media)) {
        if (!state) { return; }
        const shouldRender = isRenderedState(state);
        const ownsMediaElement = Boolean(media && stateByMedia.get(media) === state);
        clearStateTimers(state);
        if (state.taskId) {
            stateByTaskId.delete(state.taskId);
        }
        const nextIdentity = ownsMediaElement
            ? { mediaTitle: titleForMedia(media), identity: mediaIdentity(media) }
            : undefined;
        if (overlayStateCore?.resetOverlayMediaState) {
            overlayStateCore.resetOverlayMediaState(state, nextIdentity);
        } else {
            state.traceId = "";
            state.taskId = "";
            state.requestId = "";
            state.phase = "idle";
            state.message = "";
            state.speed = 0;
            state.progress = 0;
            state.failed = false;
            state.selectionKind = "";
            state.lastUiStateSignature = "";
            state.lastDownloadUiStateSignature = "";
        }
        if (ownsMediaElement) {
            state.mediaTitle = nextIdentity.mediaTitle;
            state.identity = nextIdentity.identity;
        }
        if (shouldRender) { renderCurrentMediaState(); }
    }

    function recordTelemetry(event) {
        chrome.runtime.sendMessage({
            type: "client_telemetry",
            event: {
                occurred_at: new Date().toISOString(),
                surface: "overlay",
                page_url: location.href,
                url_host: urlHost(location.href),
                url_path_kind: urlPathKind(location.href),
                ...event,
            },
        }, () => {
            void chrome.runtime.lastError;
        });
    }

    function recordUiStateFor(mediaState, action, phase, message, details) {
        const signature = `${mediaState?.traceId || ""}|${action}|${phase}|${message || ""}|${mediaState?.taskId || ""}`;
        if (signature === mediaState?.lastUiStateSignature && phase === "waiting") {
            return;
        }
        if (mediaState) {
            mediaState.lastUiStateSignature = signature;
        }
        recordTelemetry({
            event_type: "client.ui_state",
            trace_id: mediaState?.traceId || undefined,
            task_id: mediaState?.taskId || undefined,
            action,
            status: phase,
            message,
            details: {
                mediaKey: mediaState?.mediaKey,
                mediaTitle: mediaState?.mediaTitle,
                phase,
                ...details,
            },
        });
    }

    function recordDownloadUiStateFor(state, task) {
        if (!state || !task || state.phase !== "downloading") { return; }
        const progressPct = Math.round(Number(state.progress || 0) * 100);
        const speed = Number(state.speed || 0);
        const signature = `${state.taskId}|${task.eventCursor || 0}|${progressPct}|${speed}`;
        if (signature === state.lastDownloadUiStateSignature) { return; }
        state.lastDownloadUiStateSignature = signature;
        recordUiStateFor(state, "page_download_media", "downloading", formatSpeed(speed), {
            requestId: task.requestId,
            traceId: task.traceId,
            cursor: task.eventCursor,
            progress: Number(state.progress || 0),
            speed,
            receivedBytes: Number(task.receivedBytes || 0),
        });
    }

    function waitForPageMedia(timeoutMs = 1500) {
        const startedAt = performance.now();
        return new Promise((resolve) => {
            const check = () => {
                const pageMedia = window.__gd3PageMedia;
                if (pageMedia?.resolveForElement) {
                    resolve(pageMedia);
                    return;
                }
                if (performance.now() - startedAt >= timeoutMs) {
                    resolve(null);
                    return;
                }
                setTimeout(check, 50);
            };
            check();
        });
    }

    function mediaRect(media) {
        const rect = media.getBoundingClientRect();
        if (rect.width < 120 || rect.height < 80 || rect.bottom <= 0 || rect.right <= 0 || rect.top >= innerHeight || rect.left >= innerWidth) {
            return null;
        }
        return rect;
    }

    function updateVisibilityFor(selected, isFullscreen) {
        const state = selected?.media ? stateForMedia(selected.media) : null;
        const busy = isBusyState(state);
        host.classList.toggle("fullscreen-auto-hide", isFullscreen && !busy);
        host.classList.toggle("fullscreen-hover", hostHovered);
        host.classList.toggle("fullscreen-busy", isFullscreen && busy);
    }

    function revealFullscreenOverlay() {
        hostHovered = true;
        clearTimeout(fullscreenRevealTimer);
        fullscreenRevealTimer = setTimeout(() => {
            hostHovered = false;
            scheduleUpdate();
        }, 1600);
        scheduleUpdate();
    }

    // Digit slots: playing (1e9) beats readyState (1e6) beats viewport area, so the user's
    // active video always wins over a paused full-screen poster.
    function mediaScore(media, rect) {
        const playing = !media.paused && !media.ended ? 1_000_000_000 : 0;
        return playing + media.readyState * 1_000_000 + rect.width * rect.height;
    }

    function findActiveMedia() {
        let selected = null;
        for (const media of document.querySelectorAll("video")) {
            const rect = mediaRect(media);
            if (!rect) { continue; }
            const score = mediaScore(media, rect);
            if (!selected || score > selected.score) {
                selected = { media, rect, score };
            }
        }
        return selected;
    }

    function updatePosition() {
        updateQueued = false;
        if (!enabled) {
            host.style.display = "none";
            return;
        }
        const selected = findActiveMedia();
        if (!selected) {
            currentMedia = null;
            host.style.display = "none";
            return;
        }
        currentMedia = selected.media;
        stateForMedia(currentMedia);
        renderCurrentMediaState();

        // 动态调整父节点：如果某个元素进入了原生全屏（如 B站播放器），必须把悬浮按钮移到它内部，否则会被浏览器原生全屏遮挡
        const targetParent = document.fullscreenElement || document.documentElement;
        if (targetParent && host.parentNode !== targetParent) {
            targetParent.appendChild(host);
        }

        const buttonWidth = host.offsetWidth || 112;
        const buttonHeight = host.offsetHeight || 32;
        const isFullscreen = Boolean(document.fullscreenElement)
            || (selected.rect.width >= window.innerWidth * 0.98 && selected.rect.height >= window.innerHeight * 0.98);

        host.style.display = "block";
        const gap = 12;
        let left = isFullscreen
            ? window.innerWidth - buttonWidth - gap
            : selected.rect.right + gap;
        let top = selected.rect.top + gap;
        if (!isFullscreen && left + buttonWidth > innerWidth - gap) {
            left = Math.min(innerWidth - buttonWidth - gap, Math.max(gap, selected.rect.right - buttonWidth));
            top = selected.rect.top - buttonHeight - gap;
        }
        if (!isFullscreen && top < gap && selected.rect.left - buttonWidth - gap >= gap) {
            left = selected.rect.left - buttonWidth - gap;
            top = selected.rect.top;
        }
        if (!isFullscreen && top < gap) {
            top = selected.rect.bottom + gap;
        }
        left = Math.max(gap, Math.min(left, innerWidth - buttonWidth - gap));
        top = Math.max(gap, Math.min(top, innerHeight - buttonHeight - gap));

        host.style.left = `${left}px`;
        host.style.top = `${top}px`;
        updateVisibilityFor(selected, isFullscreen);
    }

    function enableOverlay() {
        enabled = true;
        const targetParent = document.fullscreenElement || document.documentElement;
        if (targetParent && host.parentNode !== targetParent) {
            targetParent.appendChild(host);
        }
        scheduleUpdate();
    }

    function disableOverlay() {
        enabled = false;
        host.style.display = "none";
        host.remove();
    }

    function scheduleUpdate() {
        if (updateQueued) { return; }
        updateQueued = true;
        requestAnimationFrame(updatePosition);
    }

    function setStatus(text, failed = false) {
        status.textContent = text;
        status.className = failed ? "status error" : "status";
        clearTimeout(resetTimer);
        if (text && !failed) {
            // Matches TERMINAL_RESET_MS in controller.ts so toast fade and button re-enable line up.
            resetTimer = setTimeout(() => { status.textContent = ""; }, 1600);
        }
    }

    function renderState(state) {
        button.disabled = false;
        button.style.cursor = isBusyState(state) ? "default" : "pointer";
        button.style.background = "";
        button.style.color = "";

        if (!state || state.phase === "idle") {
            label.textContent = "下载此媒体";
            setStatus("");
            return;
        }

        if (state.phase === "downloading") {
            label.textContent = formatSpeed(state.speed);
            setStatus("");
            const pct = Math.max(0, Math.min(100, Number(state.progress || 0) * 100));
            button.style.background = `linear-gradient(90deg, #007acc ${pct}%, rgba(255, 255, 255, 0.96) ${pct}%)`;
            button.style.color = pct > 50 ? "#fff" : "#242424";
            return;
        }

        if (state.phase === "completed") {
            label.textContent = "下载完成";
            setStatus("");
            button.style.background = `linear-gradient(90deg, #007acc 100%, rgba(255, 255, 255, 0.96) 100%)`;
            button.style.color = "#fff";
            return;
        }

        if (state.phase === "failed") {
            label.textContent = "下载失败";
            setStatus(state.message || "下载失败", true);
            return;
        }

        if (state.phase === "waiting") {
            label.textContent = state.message || "等待资源…";
            setStatus("");
            return;
        }

        if (state.phase === "resolving" || state.phase === "parsing") {
            label.textContent = "正在解析";
            setStatus("");
            return;
        }

        if (state.phase === "dispatching") {
            label.textContent = "正在发送";
            setStatus("");
            return;
        }

        if (state.phase === "starting") {
            label.textContent = "正在准备";
            setStatus("");
            return;
        }

        if (state.phase === "merging") {
            label.textContent = "正在合并";
            setStatus("");
            return;
        }

        label.textContent = "等待中";
        setStatus("");
    }

    function renderCurrentMediaState() {
        renderState(currentMedia ? stateByMedia.get(currentMedia) : null);
    }

    function formatSpeed(bytesPerSec) {
        if (bytesPerSec < 1024) return bytesPerSec + " B/s";
        if (bytesPerSec < 1024 * 1024) return (bytesPerSec / 1024).toFixed(1) + " KB/s";
        return (bytesPerSec / (1024 * 1024)).toFixed(1) + " MB/s";
    }

    function taskProgressPercent(task) {
        if (overlayStateCore?.taskProgressRatio) {
            return overlayStateCore.taskProgressRatio(task) * 100;
        }
        const progress = Number(task?.progress || 0);
        if (!Number.isFinite(progress)) { return 0; }
        return Math.max(0, Math.min(100, progress * 100));
    }

    function scheduleTerminalReset(state) {
        clearTimeout(state.terminalTimer);
        state.terminalTimer = setTimeout(() => {
            if (state.phase === "completed" || state.phase === "failed") {
                resetMediaState(null, state);
            }
        }, TERMINAL_STATE_TTL_MS);
    }

    function applyTaskToState(task, state) {
        if (!task || !state) { return false; }
        const result = overlayStateCore?.applyTaskToMediaState
            ? overlayStateCore.applyTaskToMediaState(state, task)
            : null;
        if (!result) {
            state.taskId = task.taskId || state.taskId;
            state.requestId = task.requestId || state.requestId;
            state.traceId = task.traceId || state.traceId;
            state.speed = Number(task.speed || 0);
            state.progress = taskProgressPercent(task) / 100;
            if (task.status === "completed" || task.status === "failed") {
                state.phase = task.status;
                state.message = task.status === "completed" ? "下载完成" : "下载失败";
                state.failed = task.status === "failed";
            } else if (["downloading", "starting", "parsing", "merging", "queued"].includes(task.status)) {
                state.phase = task.status;
                state.message = "";
            } else {
                state.phase = "queued";
                state.message = "";
            }
        }

        if (result?.terminal || task.status === "completed" || task.status === "failed" || task.status === "canceled") {
            recordUiStateFor(state, "page_download_media", task.status, state.message, {
                requestId: task.requestId,
                traceId: task.traceId,
                cursor: task.eventCursor,
            });
            stateByTaskId.delete(task.taskId);
            clearTimeout(state.missingTaskTimer);
            state.missingTaskTimer = 0;
            scheduleTerminalReset(state);
            if (isRenderedState(state)) { renderCurrentMediaState(); }
            return true;
        }
        clearTimeout(state.missingTaskTimer);
        state.missingTaskTimer = 0;
        recordDownloadUiStateFor(state, task);
        if (isRenderedState(state)) { renderCurrentMediaState(); }
        return true;
    }

    function applyTaskSnapshot(tasks) {
        if (!Array.isArray(tasks) || stateByTaskId.size === 0) { return; }
        for (const [taskId, state] of [...stateByTaskId.entries()]) {
            const task = tasks.find(t => t.taskId === taskId);
            applyTaskToState(task, state);
        }
    }

    function applyTaskUpdated(task) {
        if (!task?.taskId) { return; }
        const state = stateByTaskId.get(task.taskId);
        if (!state) { return; }
        applyTaskToState(task, state);
    }

    function trackDownloadProgress(state, taskId) {
        if (overlayStateCore?.bindTaskToMediaState) {
            overlayStateCore.bindTaskToMediaState(state, taskId);
        } else {
            state.taskId = taskId;
            state.phase = "queued";
            state.message = "";
            state.failed = false;
        }
        stateByTaskId.set(taskId, state);
        if (isRenderedState(state)) { renderCurrentMediaState(); }

        clearTimeout(state.missingTaskTimer);
        state.missingTaskTimer = setTimeout(() => {
            state.phase = "failed";
            state.message = "获取任务失败";
            state.failed = true;
            recordUiStateFor(state, "page_download_media", "failed", "获取任务失败", { taskId });
            stateByTaskId.delete(taskId);
            scheduleTerminalReset(state);
            if (isRenderedState(state)) { renderCurrentMediaState(); }
        }, 10000);

        chrome.runtime.sendMessage({ type: "popup_get_state" }, (snapshot) => {
            const lastError = chrome.runtime.lastError;
            if (!lastError) {
                applyTaskSnapshot(snapshot?.tasks);
            }
        });
    }

    function handleTaskSnapshotMessage(message) {
        if (message?.type === "task_snapshot") {
            applyTaskSnapshot(message.tasks);
        }
        if (message?.type === "task_updated") {
            applyTaskUpdated(message.task);
        }
    }

    async function downloadMedia() {
        const media = currentMedia && mediaRect(currentMedia)
            ? currentMedia
            : findActiveMedia()?.media;
        const mediaTitle = media ? titleForMedia(media) : document.title;
        const mediaState = media ? stateForMedia(media) : null;

        if (mediaState && isBusyState(mediaState)) {
            recordTelemetry({
                event_type: "client.click",
                trace_id: mediaState.traceId || undefined,
                task_id: mediaState.taskId || undefined,
                action: "page_download_media",
                status: "ignored",
                message: "media download already in flight",
                details: {
                    mediaKey: mediaState.mediaKey,
                    mediaTitle: mediaState.mediaTitle,
                    phase: mediaState.phase,
                },
            });
            return;
        }

        if (mediaState) {
            clearStateTimers(mediaState);
            mediaState.traceId = createTraceId();
            mediaState.taskId = "";
            mediaState.requestId = "";
            mediaState.mediaTitle = mediaTitle;
            mediaState.identity = mediaIdentity(media);
            mediaState.phase = "resolving";
            mediaState.message = "正在解析";
            mediaState.failed = false;
            mediaState.selectionKind = "";
            mediaState.lastUiStateSignature = "";
            mediaState.lastDownloadUiStateSignature = "";
        }
        renderCurrentMediaState();

        recordTelemetry({
            event_type: "client.click",
            trace_id: mediaState?.traceId,
            action: "page_download_media",
            status: "started",
            details: {
                mediaKey: mediaState?.mediaKey,
                title: mediaTitle,
                documentTitle: document.title,
                hasCurrentMedia: Boolean(currentMedia),
            },
        });
        if (!media) {
            renderState({ phase: "failed", message: "未检测到媒体" });
            return;
        }

        const pageMedia = await waitForPageMedia();
        if (!pageMedia?.resolveForElement) {
            mediaState.phase = "failed";
            mediaState.message = "扩展未就绪";
            mediaState.failed = true;
            recordUiStateFor(mediaState, "page_download_media", "failed", "扩展未就绪");
            scheduleTerminalReset(mediaState);
            renderCurrentMediaState();
            return;
        }

        // Mirror controller state on the button so the user sees a "waiting" beat instead of staring at "正在解析".
        const onState = (next, reason) => {
            if (next === "waiting") {
                mediaState.phase = "waiting";
                mediaState.message = reason || "等待资源…";
            } else if (next === "resolving") {
                mediaState.phase = "resolving";
                mediaState.message = reason || "正在解析";
            } else if (next === "dispatched") {
                mediaState.phase = "dispatching";
                mediaState.message = reason || "正在发送";
            }
            if (isRenderedState(mediaState)) { renderCurrentMediaState(); }
            recordUiStateFor(mediaState, "page_download_media", mediaState.phase, mediaState.message || reason || next);
        };

        try {
            const resolution = await pageMedia.resolveForElement(media, {
                poster: media.poster || "",
                src: media.currentSrc || media.src || "",
                title: mediaTitle,
            }, onState);
            if (!resolution || resolution.kind === "refused") {
                mediaState.phase = "failed";
                mediaState.message = resolution?.message || "未能定位媒体";
                mediaState.failed = true;
                recordUiStateFor(mediaState, "page_download_media", "failed", mediaState.message, {
                    diagnostics: resolution?.diagnostics,
                    resolutionKind: resolution?.kind,
                });
                scheduleTerminalReset(mediaState);
                if (isRenderedState(mediaState)) { renderCurrentMediaState(); }
                return;
            }
            if (resolution.kind !== "selection") {
                mediaState.phase = "failed";
                mediaState.message = "未能定位媒体";
                mediaState.failed = true;
                recordUiStateFor(mediaState, "page_download_media", "failed", "未能定位媒体", {
                    diagnostics: resolution.diagnostics,
                    resolutionKind: resolution.kind,
                });
                scheduleTerminalReset(mediaState);
                if (isRenderedState(mediaState)) { renderCurrentMediaState(); }
                return;
            }
            mediaState.phase = "dispatching";
            mediaState.message = "正在发送";
            mediaState.selectionKind = resolution.selection?.kind || "";
            if (isRenderedState(mediaState)) { renderCurrentMediaState(); }
            const result = await chrome.runtime.sendMessage({
                type: "page_download_media",
                traceId: mediaState.traceId,
                selection: resolution.selection,
                href: location.href,
                title: mediaTitle,
            });
            const ok = Boolean(result?.ok);
            pageMedia.markDispatchResult?.(media, ok, result?.message || "");
            recordTelemetry({
                event_type: "client.dispatch_result",
                trace_id: result?.traceId || mediaState.traceId,
                request_id: result?.requestId,
                task_id: result?.taskId,
                action: "page_download_media",
                status: ok ? "accepted" : "failed",
                message: result?.message || "",
                details: {
                    mediaKey: mediaState.mediaKey,
                    mediaTitle: mediaState.mediaTitle,
                    selectionKind: resolution.selection?.kind,
                },
            });

            if (ok && result?.taskId) {
                mediaState.requestId = result.requestId || mediaState.requestId;
                mediaState.traceId = result.traceId || mediaState.traceId;
                trackDownloadProgress(mediaState, result.taskId);
            } else {
                mediaState.phase = "failed";
                mediaState.message = result?.message || "发送失败";
                mediaState.failed = true;
                recordUiStateFor(mediaState, "page_download_media", "failed", mediaState.message);
                scheduleTerminalReset(mediaState);
                if (isRenderedState(mediaState)) { renderCurrentMediaState(); }
            }
        } catch (error) {
            const message = error instanceof Error ? error.message : "发送失败";
            mediaState.phase = "failed";
            mediaState.message = message;
            mediaState.failed = true;
            pageMedia.markDispatchResult?.(media, false, message);
            recordUiStateFor(mediaState, "page_download_media", "failed", message);
            scheduleTerminalReset(mediaState);
            if (isRenderedState(mediaState)) { renderCurrentMediaState(); }
        }
    }

    button.addEventListener("click", (event) => {
        event.preventDefault();
        event.stopPropagation();
        void downloadMedia();
    });
    host.addEventListener("mouseenter", () => {
        hostHovered = true;
        scheduleUpdate();
    });
    host.addEventListener("mouseleave", () => {
        hostHovered = false;
        scheduleUpdate();
    });

    addEventListener("resize", scheduleUpdate, { passive: true });
    addEventListener("scroll", scheduleUpdate, { passive: true });
    document.addEventListener("fullscreenchange", scheduleUpdate, { passive: true });
    document.addEventListener("mousemove", (event) => {
        if (!document.fullscreenElement) { return; }
        const nearRightTop = event.clientX >= window.innerWidth - 180 && event.clientY <= 96;
        if (nearRightTop) {
            revealFullscreenOverlay();
        }
    }, { passive: true });
    document.addEventListener("play", scheduleUpdate, true);
    document.addEventListener("pause", scheduleUpdate, true);
    document.addEventListener("loadedmetadata", scheduleUpdate, true);
    setInterval(scheduleUpdate, 1000);

    chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
        if (message?.type === "task_snapshot" || message?.type === "task_updated") {
            handleTaskSnapshotMessage(message);
            return;
        }
        if (message?.type !== "media_download_overlay_set_enabled") { return; }
        Boolean(message.enabled) ? enableOverlay() : disableOverlay();
        sendResponse({ ok: true });
    });

    chrome.runtime.sendMessage({ type: "page_media_overlay_state" }, (result) => {
        const lastError = chrome.runtime.lastError;
        if (!lastError && result?.enabled === false) {
            disableOverlay();
            return;
        }
        enableOverlay();
    });

    window.GhostDownloaderMediaButton = {
        installed: true,
    };
})();
