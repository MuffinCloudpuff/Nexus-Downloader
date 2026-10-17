import type {TaskError} from "./daemon";

export function messageForDaemonError(error: TaskError | undefined, fallback = "任务请求失败"): string {
  if (!error) {
    return fallback;
  }

  switch (error.code) {
    case "token_invalid":
      return "配对令牌无效，请重新配对";
    case "cookie_decryption_failed":
      return "页面凭据解密失败，请刷新页面后重试";
    case "unsupported_media":
      return "当前媒体资源暂时无法解析";
    case "idempotency_conflict":
      return "这次请求和已提交任务不一致，请重新点击下载";
    case "network_timeout":
      return "网络请求超时，请稍后重试";
    case "invalid_request":
      return error.message || "下载请求格式不正确";
    case "merge_failed":
      return "资源合并失败";
    default:
      return error.message || fallback;
  }
}

export function messageFromUnknownError(error: unknown, fallback = "请求失败"): string {
  return error instanceof Error ? error.message : fallback;
}
