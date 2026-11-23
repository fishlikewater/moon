import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { createPortal } from "react-dom";
import {
  computeMenuPosition,
  firstEnabledIndex,
  isNativeMenuTarget,
  nextEnabledIndex,
  type ContextMenuItem,
  type MenuPoint,
} from "./contextMenuCore";

export type CloseReason =
  | "escape"
  | "outside"
  | "scroll"
  | "resize"
  | "blur"
  | "tab"
  | "select";

export type ContextMenuProps = {
  open: boolean;
  position: MenuPoint;
  items: ContextMenuItem[];
  onClose: (reason: CloseReason) => void;
  ariaLabel?: string;
};

export type ContextMenuController = {
  open: boolean;
  position: MenuPoint;
  openAt: (
    event: ReactMouseEvent<HTMLElement>,
    options?: { allowNative?: boolean },
  ) => boolean;
  close: (reason?: CloseReason) => void;
};

export function useContextMenu(): ContextMenuController {
  const [state, setState] = useState<{ open: boolean; position: MenuPoint }>({
    open: false,
    position: { x: 0, y: 0 },
  });
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  const openAt = useCallback<ContextMenuController["openAt"]>(
    (event, options) => {
      const selectionText =
        typeof window === "undefined"
          ? ""
          : (window.getSelection()?.toString() ?? "");
      if (
        options?.allowNative !== true &&
        isNativeMenuTarget(event.target, selectionText)
      ) {
        return false;
      }
      event.preventDefault();
      restoreFocusRef.current =
        document.activeElement instanceof HTMLElement
          ? document.activeElement
          : null;
      setState({
        open: true,
        position: { x: event.clientX, y: event.clientY },
      });
      return true;
    },
    [],
  );

  const close = useCallback<ContextMenuController["close"]>((reason = "outside") => {
    setState((current) =>
      current.open ? { ...current, open: false } : current,
    );
    const target = restoreFocusRef.current;
    restoreFocusRef.current = null;
    if (
      reason !== "tab" &&
      target !== null &&
      target !== document.body &&
      target.isConnected
    ) {
      target.focus();
    }
  }, []);

  return { open: state.open, position: state.position, openAt, close };
}

function focusMenuItem(menu: HTMLElement, index: number): void {
  if (index < 0) {
    return;
  }
  menu
    .querySelector<HTMLElement>(`[data-menu-index="${index}"]`)
    ?.focus();
}

export function ContextMenu({
  open,
  position,
  items,
  onClose,
  ariaLabel = "操作菜单",
}: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement | null>(null);
  const closeRequestedRef = useRef(false);
  const [placement, setPlacement] = useState<MenuPoint>(position);

  const requestClose = useCallback(
    (reason: CloseReason) => {
      if (closeRequestedRef.current) {
        return;
      }
      closeRequestedRef.current = true;
      onClose(reason);
    },
    [onClose],
  );

  useEffect(() => {
    if (open) {
      closeRequestedRef.current = false;
    }
  }, [open]);

  useLayoutEffect(() => {
    if (!open) {
      return;
    }
    const menu = menuRef.current;
    if (menu === null) {
      return;
    }
    const rect = menu.getBoundingClientRect();
    setPlacement(
      computeMenuPosition(
        position,
        { width: rect.width, height: rect.height },
        { width: window.innerWidth, height: window.innerHeight },
      ),
    );
    focusMenuItem(menu, firstEnabledIndex(items));
    // 打开或锚点变化时重新测量；items 变化不重新抢焦点。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, position]);

  useEffect(() => {
    if (!open) {
      return;
    }
    const isInsideMenu = (target: EventTarget | null): boolean => {
      const menu = menuRef.current;
      return (
        menu !== null && target instanceof Node && menu.contains(target)
      );
    };
    const onPointerDown = (event: Event): void => {
      if (!isInsideMenu(event.target)) {
        requestClose("outside");
      }
    };
    const onScroll = (event: Event): void => {
      if (!isInsideMenu(event.target)) {
        requestClose("scroll");
      }
    };
    const onResize = (): void => requestClose("resize");
    const onBlur = (): void => requestClose("blur");
    const onWindowKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        event.preventDefault();
        requestClose("escape");
        return;
      }
      if (event.key === "Tab") {
        requestClose("tab");
      }
    };

    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onResize);
    window.addEventListener("blur", onBlur);
    window.addEventListener("keydown", onWindowKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onResize);
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("keydown", onWindowKeyDown);
    };
  }, [open, requestClose]);

  const onKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>): void => {
    const menu = menuRef.current;
    if (menu === null) {
      return;
    }
    const active = document.activeElement;
    const currentIndex =
      active instanceof HTMLElement && menu.contains(active)
        ? Number(active.dataset.menuIndex ?? "-1")
        : -1;

    if (event.key === "ArrowDown") {
      event.preventDefault();
      focusMenuItem(menu, nextEnabledIndex(items, currentIndex, 1));
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      focusMenuItem(menu, nextEnabledIndex(items, currentIndex, -1));
      return;
    }
    if (event.key === "Home") {
      event.preventDefault();
      focusMenuItem(menu, firstEnabledIndex(items));
      return;
    }
    if (event.key === "End") {
      event.preventDefault();
      focusMenuItem(menu, nextEnabledIndex(items, items.length, -1));
      return;
    }
    if (event.key === "Enter" || event.key === " ") {
      if (
        active instanceof HTMLElement &&
        active.getAttribute("role") === "menuitem" &&
        active.getAttribute("aria-disabled") !== "true"
      ) {
        event.preventDefault();
        active.click();
      }
    }
  };

  if (!open) {
    return null;
  }

  return createPortal(
    <div
      ref={menuRef}
      className="context-menu"
      role="menu"
      aria-label={ariaLabel}
      style={{ left: placement.x, top: placement.y }}
      onContextMenu={(event) => event.preventDefault()}
      onKeyDown={onKeyDown}
    >
      {items.map((item, index) =>
        item.kind === "separator" ? (
          <div
            key={item.id}
            className="context-menu-separator"
            role="separator"
          />
        ) : (
          <button
            key={item.id}
            type="button"
            role="menuitem"
            className={
              item.danger === true
                ? "context-menu-item is-danger"
                : "context-menu-item"
            }
            aria-disabled={item.disabled === true ? "true" : undefined}
            disabled={item.disabled === true}
            tabIndex={item.disabled === true ? -1 : 0}
            data-menu-index={index}
            onClick={() => {
              if (item.disabled === true) {
                return;
              }
              item.onSelect();
              requestClose("select");
            }}
          >
            <span className="context-menu-label">{item.label}</span>
            {item.shortcut !== undefined && (
              <span className="context-menu-shortcut" aria-hidden="true">
                {item.shortcut}
              </span>
            )}
          </button>
        ),
      )}
    </div>,
    document.body,
  );
}
