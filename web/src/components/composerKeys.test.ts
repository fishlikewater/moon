import { describe, expect, it } from "vitest";
import { composerKeyAction, type ComposerKeyInput } from "./composerKeys";

function input(overrides: Partial<ComposerKeyInput>): ComposerKeyInput {
  return {
    key: "Enter",
    shiftKey: false,
    isComposing: false,
    completionOpen: false,
    ...overrides,
  };
}

describe("composerKeyAction", () => {
  it("Enter 发送，Shift+Enter 换行", () => {
    expect(composerKeyAction(input({}))).toBe("send");
    expect(composerKeyAction(input({ shiftKey: true }))).toBe("newline");
  });

  it("输入法组合中的 Enter 不拦截", () => {
    expect(composerKeyAction(input({ isComposing: true }))).toBe("leave");
    expect(
      composerKeyAction(input({ isComposing: true, shiftKey: true })),
    ).toBe("leave");
  });

  it("补全打开时 Enter 确认补全", () => {
    expect(composerKeyAction(input({ completionOpen: true }))).toBe(
      "completion",
    );
    expect(composerKeyAction(input({ completionOpen: true, shiftKey: true }))).toBe(
      "newline",
    );
  });

  it("其他按键保持默认行为", () => {
    expect(composerKeyAction(input({ key: "a" }))).toBe("leave");
    expect(composerKeyAction(input({ key: "Escape" }))).toBe("leave");
    expect(composerKeyAction(input({ key: "ArrowUp" }))).toBe("leave");
  });
});
