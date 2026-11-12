import { describe, expect, it } from "vitest";
import { shouldOpenNew, withoutNewParam } from "./newEntry";

describe("shouldOpenNew", () => {
  it("只在 new=1 时返回 true", () => {
    expect(shouldOpenNew(new URLSearchParams("new=1"))).toBe(true);
    expect(shouldOpenNew(new URLSearchParams("?new=1"))).toBe(true);
    expect(shouldOpenNew(new URLSearchParams("new=0"))).toBe(false);
    expect(shouldOpenNew(new URLSearchParams("new="))).toBe(false);
    expect(shouldOpenNew(new URLSearchParams(""))).toBe(false);
    expect(shouldOpenNew(new URLSearchParams("other=1"))).toBe(false);
  });
});

describe("withoutNewParam", () => {
  it("只删除 new 并保留其它参数", () => {
    const next = withoutNewParam(
      new URLSearchParams("new=1&room=2&status=blocked"),
    );
    expect(next.get("new")).toBeNull();
    expect(next.get("room")).toBe("2");
    expect(next.get("status")).toBe("blocked");
  });

  it("没有 new 时保持不变", () => {
    const next = withoutNewParam(new URLSearchParams("room=2"));
    expect(next.toString()).toBe("room=2");
  });
});
