/** 房间文件面板的纯展示函数。 */

import { apiUrl } from "../runtime";

/**
 * 文件下载地址。
 *
 * 与附件链接一致，必须经 `apiUrl()` 解析运行时 base：浏览器开发走 Vite
 * 相对路径，桌面端的页面 origin 是 `tauri.localhost`，而 API 在注入的
 * `http://127.0.0.1:<port>`，裸相对地址会打到 WebView 自身。
 */
export function fileDownloadUrl(
  roomId: number,
  rootId: string,
  path: string,
): string {
  return apiUrl(
    `/api/rooms/${roomId}/files/download?root=${encodeURIComponent(rootId)}&path=${encodeURIComponent(path)}`,
  );
}
