import type {AdvancedFeatureKey} from "./types";
import {installDirectory, isFirefoxExtension} from "./browser";
import {CAT_CATCH_SCRIPT_FEATURES} from "./cat-catch";

export const DEFAULT_DAEMON_HTTP_BASE_URL = "http://127.0.0.1:14371";
export const LEGACY_WS_SERVER_URL = "ws://127.0.0.1:14370";
export const DEFAULT_SERVER_URL = DEFAULT_DAEMON_HTTP_BASE_URL;
export const EXTENSION_VERSION = chrome.runtime.getManifest().version;

export const ADVANCED_FEATURES: Array<{
  key: AdvancedFeatureKey;
  title: string;
  description: string;
  reloadRequired?: boolean;
}> = [
  {
    key: "recorder",
    title: "视频录制",
    description: "录制网页中的视频内容",
  },
  {
    key: "recorder2",
    title: "屏幕捕捉",
    description: "捕捉屏幕、窗口或标签页",
  },
  {
    key: "mobileUserAgent",
    title: "模拟手机",
    description: "模拟移动设备访问页面",
    reloadRequired: true,
  },
  {
    key: "search",
    title: "深度搜索",
    description: "深入分析页面请求资源",
    reloadRequired: CAT_CATCH_SCRIPT_FEATURES.search.reloadRequired,
  },
  {
    key: "catch",
    title: "缓存捕捉",
    description: "捕捉浏览器缓存的资源",
    reloadRequired: CAT_CATCH_SCRIPT_FEATURES.catch.reloadRequired,
  },
];

export const HELP_CONTENT: Record<string, { title: string; body: string[] }> = {
  pairing: {
    title: "如何配对桌面端",
    body: [
      "1. 启动 Nexus Downloader，保持托盘服务运行。",
      "2. 扩展会自动配对并连接桌面端。",
      "3. 如果尚未连接，可在扩展设置页点击“自动配对”重试。",
      "4. 连接状态变成“已连接”后，浏览器就能直接管理桌面任务了。",
    ],
  },
  install: {
    title: "安装说明",
    body: isFirefoxExtension()
      ? [
          "1. 打开 about:debugging#/runtime/this-firefox。",
          "2. 点击“临时载入附加组件”。",
          `3. 选择项目里的 ${installDirectory()} 目录内任意文件。`,
          "4. 保持桌面端浏览器服务开启即可正常连接。",
        ]
      : [
          "1. 打开浏览器扩展管理页并开启开发者模式。",
          "2. 选择“加载已解压的扩展程序”。",
          `3. 选择项目里的 ${installDirectory()} 目录。`,
          "4. 保持桌面端浏览器服务开启即可正常连接。",
        ],
  },
  troubleshooting: {
    title: "故障排查",
    body: [
      "1. 先确认桌面端浏览器扩展服务已经开启。",
      "2. 优先点击“自动配对”，失败时再检查配对令牌是否和桌面端一致。",
      "3. 如果状态一直断开，尝试点击设置页里的重新连接。",
      "4. 浏览器下载没有被接管时，可先确认顶部“拦截下载”开关是否开启。",
    ],
  },
};

export const PLAYBACK_RATE_OPTIONS = [0.5, 1, 1.5, 2];
