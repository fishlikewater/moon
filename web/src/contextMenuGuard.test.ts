// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { installContextMenuGuard } from "./contextMenuGuard";

let uninstall: (() => void) | null = null;

afterEach(() => {
  uninstall?.();
  uninstall = null;
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

function fireContextMenu(target: Element): MouseEvent {
  const event = new MouseEvent("contextmenu", {
    bubbles: true,
    cancelable: true,
  });
  target.dispatchEvent(event);
  return event;
}

function append(tagName: string): HTMLElement {
  const element = document.createElement(tagName);
  document.body.append(element);
  return element;
}

describe("installContextMenuGuard", () => {
  it("普通元素右键被阻止", () => {
    uninstall = installContextMenuGuard(document);
    expect(fireContextMenu(append("div")).defaultPrevented).toBe(true);
  });

  it("输入框、文本域与下拉框保留原生菜单", () => {
    uninstall = installContextMenuGuard(document);
    for (const tagName of ["input", "textarea", "select"]) {
      expect(fireContextMenu(append(tagName)).defaultPrevented, tagName).toBe(
        false,
      );
    }
  });

  it("contenteditable 及其子元素保留原生菜单", () => {
    uninstall = installContextMenuGuard(document);
    const editable = append("div");
    editable.setAttribute("contenteditable", "true");
    const child = document.createElement("span");
    editable.append(child);

    expect(fireContextMenu(editable).defaultPrevented).toBe(false);
    expect(fireContextMenu(child).defaultPrevented).toBe(false);
  });

  it("存在文本选区时保留原生菜单", () => {
    uninstall = installContextMenuGuard(document);
    vi.spyOn(window, "getSelection").mockReturnValue({
      toString: () => "选中文字",
    } as unknown as Selection);

    expect(fireContextMenu(append("div")).defaultPrevented).toBe(false);
  });

  it("卸载后不再阻止默认菜单", () => {
    const remove = installContextMenuGuard(document);
    const div = append("div");
    expect(fireContextMenu(div).defaultPrevented).toBe(true);

    remove();
    expect(fireContextMenu(div).defaultPrevented).toBe(false);
  });

  it("目标 stopPropagation 后捕获阶段仍阻止默认菜单", () => {
    uninstall = installContextMenuGuard(document);
    const div = append("div");
    div.addEventListener("contextmenu", (event) => event.stopPropagation());
    expect(fireContextMenu(div).defaultPrevented).toBe(true);
  });

  it("守卫不阻止页面自定义 handler", () => {
    uninstall = installContextMenuGuard(document);
    const div = append("div");
    let calls = 0;
    div.addEventListener("contextmenu", () => {
      calls += 1;
    });

    const event = fireContextMenu(div);
    expect(calls).toBe(1);
    expect(event.defaultPrevented).toBe(true);
  });
});
