/**
 * 智能体头像身份：首字与色相都由稳定的输入推导，不依赖后端字段。
 *
 * 色相按 1 起始的智能体 id 循环，保证相邻 id 颜色不同；具体明度/饱和度
 * 由 styles.css 的 `.avatar` 规则按明暗主题分别给出。
 */

export const AVATAR_HUES = [18, 45, 95, 150, 190, 225, 265, 320] as const;

/** 取名称首字符；空白名称回退问号。 */
export function initialOf(name: string): string {
  const trimmed = name.trim();
  if (trimmed === "") {
    return "?";
  }
  return Array.from(trimmed)[0] ?? "?";
}

/** 把智能体 id 映射到 0..7 的色相槽位；非法数字回退第一个槽位。 */
export function avatarToneIndex(id: number): number {
  if (!Number.isFinite(id)) {
    return 0;
  }
  const count = AVATAR_HUES.length;
  return (((Math.trunc(id) - 1) % count) + count) % count;
}

/** 槽位对应的 hue 角度，供测试与样式核对。 */
export function avatarHue(toneIndex: number): number {
  if (!Number.isFinite(toneIndex)) {
    return AVATAR_HUES[0];
  }
  const count = AVATAR_HUES.length;
  const normalized = ((Math.trunc(toneIndex) % count) + count) % count;
  return AVATAR_HUES[normalized] ?? AVATAR_HUES[0];
}
