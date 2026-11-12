import { describe, expect, it } from "vitest";
import {
  AVATAR_HUES,
  avatarHue,
  avatarToneIndex,
  initialOf,
} from "./avatarIdentity";

describe("avatarToneIndex", () => {
  it("同一 id 恒定", () => {
    expect(avatarToneIndex(3)).toBe(avatarToneIndex(3));
    expect(avatarToneIndex(12)).toBe(avatarToneIndex(12));
  });

  it("连续 8 个 id 覆盖 8 个色相槽位", () => {
    const tones = Array.from({ length: 8 }, (_, index) =>
      avatarToneIndex(index + 1),
    );
    expect(new Set(tones).size).toBe(8);
    expect(tones).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
  });

  it("零与负数仍落在合法槽位", () => {
    expect(avatarToneIndex(0)).toBe(7);
    expect(avatarToneIndex(-1)).toBe(6);
    expect(avatarToneIndex(Number.NaN)).toBe(0);
  });

  it("槽位对应合法色相角度", () => {
    for (const hue of AVATAR_HUES) {
      expect(hue).toBeGreaterThanOrEqual(0);
      expect(hue).toBeLessThan(360);
    }
    expect(avatarHue(0)).toBe(18);
    expect(avatarHue(7)).toBe(320);
  });
});

describe("initialOf", () => {
  it("去除空白后取首字符", () => {
    expect(initialOf(" 策划 ")).toBe("策");
    expect(initialOf("Moon")).toBe("M");
  });

  it("空白名称回退问号", () => {
    expect(initialOf("")).toBe("?");
    expect(initialOf("   ")).toBe("?");
  });

  it("多字节字符不被拆坏", () => {
    expect(initialOf("🚀 发布")).toBe("🚀");
  });
});
