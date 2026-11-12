/**
 * 运行时 API 地址解析。
 *
 * 浏览器开发由 Vite 代理（base 为空，请求走相对路径）；桌面端由 Rust
 * 初始化脚本注入 `window.__MOON_API_BASE__`，指向同进程启动的本机 HTTP 服务。
 */
declare global {
  interface Window {
    __MOON_API_BASE__?: string;
  }
}

/** 规范化 base：去掉尾部斜杠；非 http(s) 值按空处理，避免拼出非法地址。 */
export function apiBase(): string {
  const raw = window.__MOON_API_BASE__;
  if (typeof raw !== "string") {
    return "";
  }
  const trimmed = raw.trim().replace(/\/+$/, "");
  if (trimmed === "") {
    return "";
  }
  return /^https?:\/\//i.test(trimmed) ? trimmed : "";
}

/** HTTP API 地址：base 为空时保持相对路径。 */
export function apiUrl(path: string): string {
  const base = apiBase();
  return base === "" ? path : `${base}${path.startsWith("/") ? path : `/${path}`}`;
}

/** WebSocket 地址：有 base 时沿用其 host 与协议，否则使用当前页面地址。 */
export function websocketUrl(path: string): string {
  const base = apiBase();
  if (base === "") {
    const protocol = window.location.protocol === "https:" ? "wss" : "ws";
    return `${protocol}://${window.location.host}${path}`;
  }
  try {
    const url = new URL(base);
    const protocol = url.protocol === "https:" ? "wss" : "ws";
    return `${protocol}://${url.host}${path}`;
  } catch {
    const protocol = window.location.protocol === "https:" ? "wss" : "ws";
    return `${protocol}://${window.location.host}${path}`;
  }
}
