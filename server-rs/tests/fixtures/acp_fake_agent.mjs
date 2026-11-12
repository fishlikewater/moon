#!/usr/bin/env node
// 最小 ACP agent 模拟器：供 Moon 的 ACP 适配器集成测试使用。
//
// 行为由命令行参数控制：
//   --mode normal   固定输出思考/工具/用量/文本增量；prompt 形如
//                   `READ:<绝对路径>` 时向 client 发起 fs/read_text_file，
//                   并把结果（或拒绝原因）写进回复。
//   --mode record   每轮回复「第 N 轮：<prompt>」；--record <path> 把收到的
//                   每条 client 请求按行追加，供会话复用断言。
//
// 协议：newline-delimited JSON-RPC over stdio（与 ACP stdio 传输一致）。

import { appendFileSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline";

function argValue(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : null;
}

const mode = argValue("--mode") ?? "normal";
const recordPath = argValue("--record");
const pidPath = argValue("--pid-file");
const failLoad = process.argv.includes("--fail-load");
const legacyModels = process.argv.includes("--legacy-models");
const hangHandshake = process.argv.includes("--hang-handshake");

// 供超时测试断言进程确实被终止；写入失败不影响协议行为。
if (pidPath) {
  try {
    writeFileSync(pidPath, String(process.pid), "utf-8");
  } catch {
    // 忽略
  }
}

let promptCount = 0;
let nextSession = 0;
let nextServerRequestId = 1000;
/** serverRequestId → resolve；等待 client 对反向请求的响应。 */
const pendingServerRequests = new Map();

const send = (message) => {
  process.stdout.write(`${JSON.stringify(message)}\n`);
};

const record = (message) => {
  if (!recordPath) {
    return;
  }
  try {
    appendFileSync(recordPath, `${JSON.stringify(message)}\n`, "utf-8");
  } catch {
    // 记录失败不影响协议行为。
  }
};

/** fake agent 自报的能力：四种模式与两个模型，供映射测试断言。 */
const MODES = {
  currentModeId: "default",
  availableModes: [
    { id: "default", name: "Default" },
    { id: "plan", name: "Plan" },
    { id: "acceptEdits", name: "Accept Edits" },
    { id: "bypassPermissions", name: "Bypass Permissions" },
  ],
};

const CONFIG_OPTIONS = [
  {
    id: "model",
    name: "Model",
    category: "model",
    type: "select",
    currentValue: "m1",
    options: [
      { value: "m1", name: "Model 1" },
      { value: "m2", name: "Model 2" },
    ],
  },
  {
    id: "reasoning_effort",
    name: "Reasoning Effort",
    category: "thinking",
    type: "select",
    currentValue: "medium",
    options: [
      { value: "low", name: "Low" },
      { value: "medium", name: "Medium" },
      { value: "high", name: "High" },
    ],
  },
];

/** 旧式 models 契约（pre-v1 agent）：无 configOptions，模型走 session/set_model。 */
const LEGACY_MODELS = {
  currentModelId: "m1",
  availableModels: [
    { modelId: "m1", name: "Model 1" },
    { modelId: "m2", name: "Model 2" },
  ],
};

const capabilities = () => ({
  modes: JSON.parse(JSON.stringify(MODES)),
  ...(legacyModels
    ? { models: JSON.parse(JSON.stringify(LEGACY_MODELS)) }
    : { configOptions: JSON.parse(JSON.stringify(CONFIG_OPTIONS)) }),
});

const readline = createInterface({ input: process.stdin });
readline.on("line", (line) => {
  const trimmed = line.trim();
  if (!trimmed) {
    return;
  }
  let message;
  try {
    message = JSON.parse(trimmed);
  } catch {
    return;
  }
  handleMessage(message);
});

function handleMessage(message) {
  record(message);

  // client 对反向请求的响应；envelope 必须是 JSON-RPC 2.0。
  if (message.id !== undefined && message.method === undefined) {
    const pending = pendingServerRequests.get(message.id);
    if (pending) {
      pendingServerRequests.delete(message.id);
      if (message.jsonrpc !== "2.0") {
        pending({
          error: { code: -32600, message: `反向请求响应缺少 jsonrpc 2.0 标识：${JSON.stringify(message)}` },
        });
      } else {
        pending(message);
      }
    }
    return;
  }

  switch (message.method) {
    case "initialize": {
      if (hangHandshake) {
        // 握手挂起：保活不响应，供探测超时测试验证 client 主动终止。
        setInterval(() => {}, 1000);
        return;
      }
      const loadSession = !process.argv.includes("--no-load-session");
      send({
        jsonrpc: "2.0",
        id: message.id,
        result: {
          protocolVersion: 1,
          agentCapabilities: { loadSession },
        },
      });
      return;
    }
    case "session/new":
      nextSession += 1;
      send({
        jsonrpc: "2.0",
        id: message.id,
        result: { sessionId: `sess-fake-${nextSession}`, ...capabilities() },
      });
      return;
    case "session/load":
      if (failLoad) {
        send({
          jsonrpc: "2.0",
          id: message.id,
          error: { code: -32002, message: "fake agent 拒绝加载会话" },
        });
        return;
      }
      send({
        jsonrpc: "2.0",
        id: message.id,
        result: {
          sessionId: message.params?.sessionId ?? "sess-fake-loaded",
          ...capabilities(),
        },
      });
      return;
    case "session/set_mode":
      send({ jsonrpc: "2.0", id: message.id, result: {} });
      return;
    case "session/set_config_option":
      send({
        jsonrpc: "2.0",
        id: message.id,
        result: { configOptions: JSON.parse(JSON.stringify(CONFIG_OPTIONS)) },
      });
      return;
    case "session/set_model":
      send({ jsonrpc: "2.0", id: message.id, result: {} });
      return;
    case "session/prompt":
      runPrompt(message);
      return;
    default:
      if (message.id !== undefined) {
        send({
          jsonrpc: "2.0",
          id: message.id,
          error: { code: -32601, message: `fake agent 未实现：${message.method}` },
        });
      }
  }
}

function update(sessionId, body) {
  send({
    jsonrpc: "2.0",
    method: "session/update",
    params: { sessionId, update: body },
  });
}

function requestReadFile(promptMessage, path) {
  return new Promise((resolve) => {
    const id = nextServerRequestId++;
    pendingServerRequests.set(id, resolve);
    send({
      jsonrpc: "2.0",
      id,
      method: "fs/read_text_file",
      params: { sessionId: promptMessage.params?.sessionId, path },
    });
  });
}

function requestServerMethod(method, params) {
  return new Promise((resolve) => {
    const id = nextServerRequestId++;
    pendingServerRequests.set(id, resolve);
    send({ jsonrpc: "2.0", id, method, params });
  });
}

async function runPrompt(message) {
  const sessionId = message.params?.sessionId ?? "sess-fake-unknown";
  const promptText =
    message.params?.prompt
      ?.filter((block) => block.type === "text")
      .map((block) => block.text)
      .join("\n") ?? "";
  promptCount += 1;

  const respond = (stopReason) =>
    send({ jsonrpc: "2.0", id: message.id, result: { stopReason } });
  const chunks = (...texts) => {
    for (const text of texts) {
      update(sessionId, {
        sessionUpdate: "agent_message_chunk",
        content: { type: "text", text },
      });
    }
  };

  if (mode === "hang") {
    // 超时测试：握手正常完成，收到 prompt 后保持静默并保活，
    // 确保测试验证的是 client 主动终止，而不是 stdin EOF 后自行退出。
    setInterval(() => {}, 1000);
    return;
  }

  if (mode === "record") {
    chunks(`第 ${promptCount} 轮：${promptText}`);
    respond("end_turn");
    return;
  }

  const readMatch = /^READ:(.+)$/m.exec(promptText);
  if (readMatch) {
    const response = await requestReadFile(message, readMatch[1].trim());
    if (response.error) {
      chunks(`READ_FAIL:${response.error.message ?? "未知错误"}`);
    } else {
      const content = response.result?.content ?? "";
      chunks(`READ_OK:${content.slice(0, 200)}`);
    }
    respond("end_turn");
    return;
  }

  const writeMatch = /^WRITE:(.+?)::(.*)$/m.exec(promptText);
  if (writeMatch) {
    const [, path, content] = writeMatch;
    const response = await requestServerMethod("fs/write_text_file", {
      sessionId,
      path: path.trim(),
      content,
    });
    if (response.error) {
      chunks(`WRITE_FAIL:${response.error.message ?? "未知错误"}`);
    } else {
      chunks("WRITE_OK");
    }
    respond("end_turn");
    return;
  }

  if (/^PERMISSION$/m.test(promptText)) {
    const response = await requestServerMethod("session/request_permission", {
      sessionId,
      toolCall: { toolCallId: "call-perm", status: "pending" },
      options: [
        { optionId: "allow-1", kind: "allow_once", name: "允许" },
        { optionId: "reject-1", kind: "reject_once", name: "拒绝" },
      ],
    });
    const outcome = response.result?.outcome?.outcome;
    if (outcome === "selected") {
      chunks(`PERM_OK:${response.result.outcome.optionId}`);
    } else {
      chunks("PERM_CANCELLED");
    }
    respond("end_turn");
    return;
  }

  // 固定序列：思考 → 工具调用 → 用量 → 文本增量 → 结束。
  update(sessionId, {
    sessionUpdate: "agent_thought_chunk",
    content: { type: "text", text: "正在思考" },
  });
  update(sessionId, {
    sessionUpdate: "tool_call",
    toolCallId: "call-1",
    title: "列出文件",
    kind: "read",
    status: "in_progress",
  });
  update(sessionId, {
    sessionUpdate: "tool_call_update",
    toolCallId: "call-1",
    status: "completed",
  });
  update(sessionId, {
    sessionUpdate: "usage_update",
    used: 42,
    size: 1000,
  });
  chunks("你好，", "我是自定义 ACP agent");
  respond("end_turn");
}
