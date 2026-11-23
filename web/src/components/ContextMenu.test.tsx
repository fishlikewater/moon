// @vitest-environment jsdom
import {
  cleanup,
  createEvent,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  ContextMenu,
  useContextMenu,
  type CloseReason,
} from "./ContextMenu";
import type { ContextMenuItem } from "./contextMenuCore";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function item(
  id: string,
  options: { disabled?: boolean; danger?: boolean; onSelect?: () => void } = {},
): ContextMenuItem {
  return {
    kind: "item",
    id,
    label: id,
    disabled: options.disabled,
    danger: options.danger,
    onSelect: options.onSelect ?? (() => {}),
  };
}

function Harness({
  items,
  onCloseSpy,
  allowNative,
}: {
  items: ContextMenuItem[];
  onCloseSpy?: (reason: CloseReason) => void;
  allowNative?: boolean;
}) {
  const menu = useContextMenu();
  return (
    <div>
      <button
        type="button"
        onContextMenu={(event) => {
          menu.openAt(
            event,
            allowNative === true ? { allowNative: true } : undefined,
          );
        }}
      >
        触发
      </button>
      <input
        aria-label="输入"
        onContextMenu={(event) => {
          menu.openAt(
            event,
            allowNative === true ? { allowNative: true } : undefined,
          );
        }}
      />
      <ContextMenu
        open={menu.open}
        position={menu.position}
        items={items}
        onClose={(reason) => {
          onCloseSpy?.(reason);
          menu.close(reason);
        }}
      />
    </div>
  );
}

function StickyHarness({
  items,
  onCloseSpy,
}: {
  items: ContextMenuItem[];
  onCloseSpy: (reason: CloseReason) => void;
}) {
  const menu = useContextMenu();
  return (
    <div>
      <button
        type="button"
        onContextMenu={(event) => {
          menu.openAt(event);
        }}
      >
        触发
      </button>
      <ContextMenu
        open={menu.open}
        position={menu.position}
        items={items}
        onClose={onCloseSpy}
      />
    </div>
  );
}

function openMenu(): HTMLElement {
  fireEvent.contextMenu(screen.getByRole("button", { name: "触发" }), {
    clientX: 10,
    clientY: 20,
  });
  return screen.getByRole("menu", { name: "操作菜单" });
}

describe("ContextMenu 打开与选择", () => {
  it("右键打开菜单、聚焦首个可用项、选中后关闭", () => {
    const onSelect = vi.fn();
    render(<Harness items={[item("复制", { onSelect })]} />);

    const trigger = screen.getByRole("button", { name: "触发" });
    const event = createEvent.contextMenu(trigger, {
      clientX: 10,
      clientY: 20,
    });
    fireEvent(trigger, event);

    expect(event.defaultPrevented).toBe(true);
    const menu = screen.getByRole("menu", { name: "操作菜单" });
    expect(menu.parentElement).toBe(document.body);

    const menuItem = screen.getByRole("menuitem", { name: "复制" });
    expect(document.activeElement).toBe(menuItem);

    fireEvent.click(menuItem);

    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("输入框目标不接管右键，也不阻止默认行为", () => {
    render(<Harness items={[item("复制")]} />);

    const input = screen.getByRole("textbox", { name: "输入" });
    const event = createEvent.contextMenu(input, {
      clientX: 10,
      clientY: 20,
    });
    fireEvent(input, event);

    expect(event.defaultPrevented).toBe(false);
    expect(screen.queryByRole("menu")).toBeNull();
  });
});

describe("ContextMenu 键盘导航", () => {
  const items: ContextMenuItem[] = [
    item("A"),
    { kind: "separator", id: "sep" },
    item("禁用", { disabled: true }),
    item("B"),
  ];

  it("方向键跳过分隔符与禁用项并循环", () => {
    render(<Harness items={items} />);
    const menu = openMenu();
    const first = screen.getByRole("menuitem", { name: "A" });
    const last = screen.getByRole("menuitem", { name: "B" });
    expect(document.activeElement).toBe(first);

    fireEvent.keyDown(first, { key: "ArrowDown" });
    expect(document.activeElement).toBe(last);

    fireEvent.keyDown(last, { key: "ArrowDown" });
    expect(document.activeElement).toBe(first);

    fireEvent.keyDown(first, { key: "ArrowUp" });
    expect(document.activeElement).toBe(last);

    fireEvent.keyDown(last, { key: "Home" });
    expect(document.activeElement).toBe(first);

    fireEvent.keyDown(first, { key: "End" });
    expect(document.activeElement).toBe(last);

    expect(menu.contains(document.activeElement)).toBe(true);
  });

  it("Enter 激活当前项并关闭", () => {
    const onSelect = vi.fn();
    render(<Harness items={[item("A", { onSelect })]} />);
    openMenu();

    fireEvent.keyDown(document.activeElement as HTMLElement, {
      key: "Enter",
    });

    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("Space 激活当前项并关闭", () => {
    const onSelect = vi.fn();
    render(<Harness items={[item("A", { onSelect })]} />);
    openMenu();

    fireEvent.keyDown(document.activeElement as HTMLElement, { key: " " });

    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("menu")).toBeNull();
  });
});

describe("ContextMenu 焦点归还", () => {
  it("Esc 关闭后焦点回到触发元素", () => {
    render(<Harness items={[item("A")]} />);
    const trigger = screen.getByRole("button", { name: "触发" });
    trigger.focus();
    openMenu();

    fireEvent.keyDown(document.activeElement as HTMLElement, {
      key: "Escape",
    });

    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("外部 pointerdown 关闭后焦点回到触发元素", () => {
    render(<Harness items={[item("A")]} />);
    const trigger = screen.getByRole("button", { name: "触发" });
    trigger.focus();
    openMenu();

    fireEvent.pointerDown(document.body);

    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("Tab 关闭但不抢走浏览器默认焦点目标", () => {
    render(<Harness items={[item("A")]} />);
    const trigger = screen.getByRole("button", { name: "触发" });
    trigger.focus();
    openMenu();

    fireEvent.keyDown(document.activeElement as HTMLElement, { key: "Tab" });

    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).not.toBe(trigger);
  });
});

describe("ContextMenu 关闭路径", () => {
  it("多种关闭事件只触发一次回调", () => {
    const onCloseSpy = vi.fn();
    render(<Harness items={[item("A")]} onCloseSpy={onCloseSpy} />);
    openMenu();

    fireEvent.pointerDown(document.body);
    fireEvent.scroll(document.body);
    fireEvent(window, new Event("resize"));
    fireEvent(window, new Event("blur"));

    expect(onCloseSpy).toHaveBeenCalledTimes(1);
    expect(onCloseSpy).toHaveBeenCalledWith("outside");
  });

  it.each([
    ["scroll", () => fireEvent.scroll(document.body)],
    ["resize", () => fireEvent(window, new Event("resize"))],
    ["blur", () => fireEvent(window, new Event("blur"))],
  ])("%s 触发对应关闭原因", (reason, dispatch) => {
    const onCloseSpy = vi.fn();
    render(<Harness items={[item("A")]} onCloseSpy={onCloseSpy} />);
    openMenu();

    dispatch();

    expect(onCloseSpy).toHaveBeenCalledWith(reason);
    expect(screen.queryByRole("menu")).toBeNull();
  });
});

describe("ContextMenu 边界场景", () => {
  it("仅含禁用项时 Esc 仍关闭", () => {
    render(<Harness items={[item("禁用", { disabled: true })]} />);
    openMenu();

    fireEvent.keyDown(document.body, { key: "Escape" });

    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("仅含禁用项时 Tab 仍关闭", () => {
    render(<Harness items={[item("禁用", { disabled: true })]} />);
    openMenu();

    fireEvent.keyDown(document.body, { key: "Tab" });

    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("父组件未关闭时连续关闭事件只回调一次", () => {
    const onCloseSpy = vi.fn();
    render(<StickyHarness items={[item("A")]} onCloseSpy={onCloseSpy} />);
    fireEvent.contextMenu(screen.getByRole("button", { name: "触发" }), {
      clientX: 10,
      clientY: 20,
    });
    expect(screen.getByRole("menu")).not.toBeNull();

    fireEvent.pointerDown(document.body);
    fireEvent.scroll(document.body);

    expect(onCloseSpy).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("menu")).not.toBeNull();
  });

  it("菜单内部滚动与按下不关闭", () => {
    const onCloseSpy = vi.fn();
    render(<Harness items={[item("A")]} onCloseSpy={onCloseSpy} />);
    const menu = openMenu();

    fireEvent.scroll(menu);
    fireEvent.pointerDown(menu);

    expect(screen.getByRole("menu")).not.toBeNull();
    expect(onCloseSpy).not.toHaveBeenCalled();
  });

  it("存在文本选区时不接管右键", () => {
    const text = document.createElement("p");
    text.textContent = "已选中的文字";
    document.body.append(text);
    const range = document.createRange();
    range.selectNodeContents(text);
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);

    try {
      render(<Harness items={[item("复制")]} />);
      const trigger = screen.getByRole("button", { name: "触发" });
      const event = createEvent.contextMenu(trigger, {
        clientX: 10,
        clientY: 20,
      });
      fireEvent(trigger, event);

      expect(event.defaultPrevented).toBe(false);
      expect(screen.queryByRole("menu")).toBeNull();
    } finally {
      selection?.removeAllRanges();
      text.remove();
    }
  });

  it("allowNative 时输入框可强制打开自定义菜单", () => {
    render(<Harness items={[item("复制")]} allowNative />);

    const input = screen.getByRole("textbox", { name: "输入" });
    const event = createEvent.contextMenu(input, {
      clientX: 10,
      clientY: 20,
    });
    fireEvent(input, event);

    expect(event.defaultPrevented).toBe(true);
    expect(screen.getByRole("menu")).not.toBeNull();
  });
});

describe("ContextMenu 定位与语义", () => {
  it("菜单尺寸越界时夹紧到视口内", () => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
      width: 200,
      height: 160,
      x: 0,
      y: 0,
      top: 0,
      left: 0,
      right: 200,
      bottom: 160,
      toJSON: () => ({}),
    } as DOMRect);
    render(<Harness items={[item("A")]} />);

    fireEvent.contextMenu(screen.getByRole("button", { name: "触发" }), {
      clientX: 900,
      clientY: 700,
    });
    const menu = screen.getByRole("menu", { name: "操作菜单" });

    expect(menu.style.left).toBe("700px");
    expect(menu.style.top).toBe("540px");
  });

  it("输出 role 语义与禁用态", () => {
    render(
      <Harness
        items={[item("A"), { kind: "separator", id: "sep" }, item("禁用", { disabled: true })]}
      />,
    );
    openMenu();

    const disabled = screen.getByRole("menuitem", { name: "禁用" });
    expect(disabled.getAttribute("aria-disabled")).toBe("true");
    expect(disabled.getAttribute("tabindex")).toBe("-1");
    expect((disabled as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getAllByRole("menuitem")).toHaveLength(2);
  });

  it("样式 class 契约稳定且不写内联颜色", () => {
    render(
      <Harness
        items={[
          item("普通"),
          item("删除", { danger: true }),
          { kind: "separator", id: "sep" },
        ]}
      />,
    );

    const menu = openMenu();

    expect(menu.className).toContain("context-menu");
    expect(screen.getByRole("menuitem", { name: "普通" }).className).toContain(
      "context-menu-item",
    );
    expect(screen.getByRole("menuitem", { name: "删除" }).className).toContain(
      "is-danger",
    );
    expect(screen.getByRole("separator").className).toContain(
      "context-menu-separator",
    );
    expect(menu.getAttribute("style")).not.toContain("color");
  });
});
