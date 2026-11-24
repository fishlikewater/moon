/** 应用入口的全局右键守卫：阻止非原生目标的默认菜单，不负责打开自定义菜单。 */

import { isNativeMenuTarget } from "./components/contextMenuCore";

/**
 * 在 `document` 捕获阶段抑制 WebView 默认右键菜单。
 *
 * 输入框、可编辑区域与存在文本选区的目标保留原生菜单，复制/粘贴/全选由此兜底；
 * 显式接入 `useContextMenu` 的区域仍由页面自身打开自定义菜单。
 */
export function installContextMenuGuard(doc: Document = document): () => void {
  const view = doc.defaultView;
  const onContextMenu = (event: MouseEvent): void => {
    const selection = view?.getSelection()?.toString() ?? "";
    if (!isNativeMenuTarget(event.target, selection)) {
      event.preventDefault();
    }
  };
  doc.addEventListener("contextmenu", onContextMenu, true);
  return () => doc.removeEventListener("contextmenu", onContextMenu, true);
}
