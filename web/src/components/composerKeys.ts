/**
 * 房间消息输入框的 Enter 判定。
 *
 * - Enter（无 Shift、非输入法组合）在补全打开时确认补全，否则发送；
 * - Shift+Enter 交给 textarea 默认换行；
 * - 输入法组合中的 Enter 不拦截，避免中文候选上屏时误发送。
 */
export type ComposerKeyAction = "send" | "newline" | "completion" | "leave";

export interface ComposerKeyInput {
  key: string;
  shiftKey: boolean;
  isComposing: boolean;
  completionOpen: boolean;
}

export function composerKeyAction(input: ComposerKeyInput): ComposerKeyAction {
  if (input.key !== "Enter" || input.isComposing) {
    return "leave";
  }
  if (input.shiftKey) {
    return "newline";
  }
  return input.completionOpen ? "completion" : "send";
}
