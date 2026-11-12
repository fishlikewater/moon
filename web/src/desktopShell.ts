/**
 * 桌面壳判定。
 *
 * 桌面端由 Rust 初始化脚本注入 `window.__MOON_API_BASE__`（本机 HTTP 地址），
 * 浏览器开发模式没有该值。窗口控制按钮只应在桌面壳内渲染。
 */
export function isDesktopShell(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.__MOON_API_BASE__ === "string"
  );
}
