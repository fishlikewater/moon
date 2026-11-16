import { useEffect, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import type { Agent } from "@moon/shared";
import { api, errorMessage } from "../api/client";
import { AgentAvatar } from "../components/AgentAvatar";
import { PageHeader } from "../components/layout/PageHeader";
import {
  createDirectChatEntry,
  resolveDirectChatEntry,
  shouldCreateDirectChat,
  shouldRedirectToDirectChat,
  type DirectChatEntryPhase,
} from "./directChatEntry";

/** 地址里的智能体 id：只接受正整数。 */
function parseAgentId(raw: string | undefined): number | null {
  if (raw === undefined || !/^\d+$/.test(raw)) {
    return null;
  }
  const value = Number(raw);
  return Number.isSafeInteger(value) && value > 0 ? value : null;
}

/**
 * 直聊跳板：`/chat/:agentId`。
 *
 * 渲染路径只读——智能体已开聊时跳转到 `/chat/room/:roomId`，未开聊时渲染
 * 引导卡；创建只由「发起对话」按钮触发，刷新或直达不会隐式建房间。
 */
export default function DirectChatEntryPage() {
  const { agentId: rawAgentId } = useParams<{ agentId: string }>();
  const agentId = parseAgentId(rawAgentId);
  const navigate = useNavigate();
  const [agent, setAgent] = useState<Agent | null>(null);
  const [phase, setPhase] = useState<DirectChatEntryPhase>("loading");
  const [targetRoomId, setTargetRoomId] = useState<number | null>(null);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setPhase("loading");
    setError(null);
    setAgent(null);
    setTargetRoomId(null);
    void (async () => {
      const result = await resolveDirectChatEntry(agentId, {
        listAgents: api.agents.list,
        getDirectChat: api.agents.directChat.get,
      });
      if (cancelled) {
        return;
      }
      if (result.phase === "error") {
        setPhase("error");
        setError(result.message);
        return;
      }
      setAgent(result.agent);
      setPhase(result.phase);
      if (result.phase === "navigate") {
        setTargetRoomId(result.roomId);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [agentId]);
  // 命中已有直聊后跳转；effect 承担副作用，渲染路径保持只读。
  useEffect(() => {
    if (shouldRedirectToDirectChat(phase) && targetRoomId !== null) {
      navigate(`/chat/room/${targetRoomId}`, { replace: true });
    }
  }, [phase, targetRoomId, navigate]);

  const startChat = async () => {
    if (agentId === null || !shouldCreateDirectChat({ phase, creating })) {
      return;
    }
    setCreating(true);
    setError(null);
    try {
      const roomId = await createDirectChatEntry(agentId, {
        create: api.agents.directChat.create,
      });
      setTargetRoomId(roomId);
      setPhase("navigate");
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setCreating(false);
    }
  };

  if (phase === "loading" || phase === "navigate") {
    return (
      <section className="page">
        <p className="muted">
          {phase === "navigate" ? "正在打开对话…" : "加载中…"}
        </p>
      </section>
    );
  }

  return (
    <section className="page">
      <PageHeader
        title="发起直聊"
        subtitle="与单个智能体一对一会话，不需要创建房间。"
        backTo="/agents"
        backLabel="返回智能体"
      />

      {phase === "error" || agent === null ? (
        <p className="banner error" role="alert">
          {error ?? "智能体不可用"}
        </p>
      ) : (
        <div className="direct-entry">
          <div className="direct-entry-head">
            <AgentAvatar id={agent.id} name={agent.name} size="lg" />
            <div className="direct-entry-body">
              <h2 className="direct-entry-name">{agent.name}</h2>
              <p className="direct-entry-badges">
                <span className="badge badge-harness">{agent.harness}</span>
                <span className="chip chip-mono">
                  {agent.modelConfig?.model ?? "默认"}
                </span>
              </p>
              <p className="agent-persona">{agent.persona}</p>
            </div>
          </div>

          <p className="direct-entry-note">
            直聊会为 TA 分配独立的沙盒工作目录，不影响该智能体在其他房间使用的
            工作区；消息、会话与运行记录走与房间相同的链路。直接输入即可开始，
            无需 @ 成员。
          </p>

          {error !== null && (
            <p className="banner error" role="alert">
              {error}
            </p>
          )}

          <div className="direct-entry-actions">
            <button
              type="button"
              className="btn btn-primary"
              onClick={() => void startChat()}
              disabled={!shouldCreateDirectChat({ phase, creating })}
            >
              {creating ? "创建中…" : "发起对话"}
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
