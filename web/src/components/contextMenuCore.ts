export type MenuPoint = {
  x: number;
  y: number;
};

export type MenuSize = {
  width: number;
  height: number;
};

export type ViewportSize = {
  width: number;
  height: number;
};

export type ContextMenuItem =
  | {
      kind: "item";
      id: string;
      label: string;
      disabled?: boolean;
      danger?: boolean;
      shortcut?: string;
      onSelect: () => void;
    }
  | {
      kind: "separator";
      id: string;
    };

export const MENU_VIEWPORT_MARGIN = 8;

function placeAxis(
  anchor: number,
  size: number,
  viewport: number,
  margin: number,
): number {
  const max = viewport - size - margin;
  if (max < margin) {
    return margin;
  }
  const preferred =
    anchor + size > viewport - margin ? anchor - size : anchor;
  return Math.min(Math.max(preferred, margin), max);
}

/** 以右键点左上角为默认位置，越界时先翻转、再夹紧到视口内。 */
export function computeMenuPosition(
  anchor: MenuPoint,
  menuSize: MenuSize,
  viewport: ViewportSize,
  margin = MENU_VIEWPORT_MARGIN,
): MenuPoint {
  return {
    x: placeAxis(anchor.x, menuSize.width, viewport.width, margin),
    y: placeAxis(anchor.y, menuSize.height, viewport.height, margin),
  };
}

function isEnabledItem(item: ContextMenuItem | undefined): boolean {
  return item !== undefined && item.kind === "item" && item.disabled !== true;
}

export function firstEnabledIndex(items: ContextMenuItem[]): number {
  for (let index = 0; index < items.length; index += 1) {
    if (isEnabledItem(items[index])) {
      return index;
    }
  }
  return -1;
}

/** 从 current 按 delta 循环移动；current 可为 -1 或 items.length 表示边界起点。 */
export function nextEnabledIndex(
  items: ContextMenuItem[],
  current: number,
  delta: number,
): number {
  if (items.length === 0) {
    return -1;
  }
  const step = delta >= 0 ? 1 : -1;
  let index = current;
  for (let visited = 0; visited < items.length; visited += 1) {
    index = (index + step + items.length) % items.length;
    if (isEnabledItem(items[index])) {
      return index;
    }
  }
  return -1;
}

type MenuTargetLike = {
  tagName?: unknown;
  getAttribute?: (name: string) => string | null;
  parentElement?: MenuTargetLike | null;
};

function asTargetLike(target: EventTarget | null): MenuTargetLike | null {
  if (target === null || typeof target !== "object") {
    return null;
  }
  return target as MenuTargetLike;
}

/** 输入框、可编辑区域或存在文本选区时，应保留浏览器原生右键菜单。 */
export function isNativeMenuTarget(
  target: EventTarget | null,
  selectionText = "",
): boolean {
  if (selectionText.trim().length > 0) {
    return true;
  }
  let node = asTargetLike(target);
  while (node !== null) {
    const tagName =
      typeof node.tagName === "string" ? node.tagName.toUpperCase() : "";
    if (tagName === "INPUT" || tagName === "TEXTAREA" || tagName === "SELECT") {
      return true;
    }
    const editable =
      typeof node.getAttribute === "function"
        ? node.getAttribute("contenteditable")
        : null;
    if (editable !== null && editable.toLowerCase() !== "false") {
      return true;
    }
    node = node.parentElement ?? null;
  }
  return false;
}
