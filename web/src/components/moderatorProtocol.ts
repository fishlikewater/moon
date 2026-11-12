/**
 * 主持人编排协议块的展示折叠。
 *
 * 引擎需要保存模型原始输出以供审计，但聊天区和时间线不应直接展示 JSON。
 * 这里只识别 `assign` / `done` 两种协议块；无法识别的内容原样返回，
 * 避免把普通代码块或成员产出误折叠。
 */

const FENCED_JSON_BLOCK =
  /```[ \t]*(?:json)?[ \t]*\r?\n([\s\S]*?)```/gi;

interface Assignment {
  agentId: number | null;
  instruction: string;
  kind: "work" | "review";
  expectedOutput: string | null;
}

type ProtocolBlock =
  | { type: "assign"; assignments: Assignment[] }
  | { type: "done"; summary: string; verification: string[] };

export type AgentNameLookup = (agentId: number) => string | undefined;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readAssignments(value: unknown): ProtocolBlock | null {
  if (!isRecord(value) || value.type !== "assign") {
    return null;
  }
  if (!Array.isArray(value.assignments)) {
    return null;
  }

  const assignments: Assignment[] = [];
  for (const item of value.assignments) {
    if (!isRecord(item) || typeof item.instruction !== "string") {
      return null;
    }
    assignments.push({
      agentId:
        typeof item.agentId === "number" && Number.isSafeInteger(item.agentId)
          ? item.agentId
          : null,
      instruction: item.instruction.trim(),
      kind: item.kind === "review" ? "review" : "work",
      expectedOutput:
        typeof item.expectedOutput === "string" &&
        item.expectedOutput.trim() !== ""
          ? item.expectedOutput.trim()
          : null,
    });
  }

  return assignments.length === 0
    ? null
    : { type: "assign", assignments };
}

function readDone(value: unknown): ProtocolBlock | null {
  if (!isRecord(value) || value.type !== "done") {
    return null;
  }
  if (typeof value.summary !== "string" || value.summary.trim() === "") {
    return null;
  }

  const verification = Array.isArray(value.verification)
    ? value.verification.filter(
        (item): item is string =>
          typeof item === "string" && item.trim() !== "",
      )
    : [];

  return {
    type: "done",
    summary: value.summary.trim(),
    verification: verification.map((item) => item.trim()),
  };
}

function parseProtocolBlock(body: string): ProtocolBlock | null {
  let value: unknown;
  try {
    value = JSON.parse(body);
  } catch {
    return null;
  }
  return readAssignments(value) ?? readDone(value);
}

function summarize(
  block: ProtocolBlock,
  lookup: AgentNameLookup | undefined,
): string {
  if (block.type === "done") {
    const evidence =
      block.verification.length === 0
        ? []
        : [
            "验收证据：",
            ...block.verification.map((item, index) => `${index + 1}. ${item}`),
          ];
    return [`主持人验收：${block.summary}`, ...evidence].join("\n");
  }

  const lines = block.assignments.map((assignment, index) => {
    const name =
      assignment.agentId === null
        ? "未知成员"
        : (lookup?.(assignment.agentId) ?? `智能体#${assignment.agentId}`);
    const kind = assignment.kind === "review" ? "复核" : "执行";
    const expected =
      assignment.expectedOutput === null
        ? ""
        : ` → ${assignment.expectedOutput}`;
    return `${index + 1}. [${kind}] ${name}：${assignment.instruction}${expected}`;
  });

  return ["主持人安排本轮工作：", ...lines].join("\n");
}

/**
 * 将主持人协议块折叠为可读摘要；非协议文本原样返回。
 *
 * 同一回复中的多个协议块全部从展示文本剥离，摘要取最后一个可识别块，
 * 与服务端“最后一条指令生效”的解析语义保持一致。
 */
export function formatModeratorTranscript(
  text: string,
  lookup?: AgentNameLookup,
  isModerator = true,
): string {
  if (!isModerator) {
    return text;
  }

  let lastBlock: ProtocolBlock | null = null;
  let stripped = text;
  const pattern = new RegExp(FENCED_JSON_BLOCK.source, "gi");

  for (const match of text.matchAll(pattern)) {
    const body = match[1];
    if (body === undefined || match[0] === undefined) {
      continue;
    }
    const block = parseProtocolBlock(body);
    if (block === null) {
      continue;
    }
    lastBlock = block;
    stripped = stripped.replace(match[0], "");
  }

  if (lastBlock === null) {
    return text;
  }

  const remainder = stripped.trim();
  const summary = summarize(lastBlock, lookup);
  return remainder === "" ? summary : `${remainder}\n\n${summary}`;
}
