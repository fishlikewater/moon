import { describe, expect, it } from "vitest";
import type { CustomHarness } from "@moon/shared";
import { acpDraftModels, acpDraftReasoningLevels } from "./agentBuilderDraft";

function customHarness(overrides: Partial<CustomHarness>): CustomHarness {
  return {
    id: 7,
    name: "本地 ACP",
    command: "agent-acp",
    args: [],
    env: {},
    createdAt: 1,
    agentCount: 0,
    capabilities: null,
    probedAt: null,
    ...overrides,
  };
}

describe("acpDraftModels", () => {
  it("把探测到的模型映射为草稿候选", () => {
    const models = acpDraftModels(
      customHarness({
        capabilities: {
          models: [
            { value: "acp/model-a", name: "Model A" },
            { value: "acp/model-b", name: "Model B" },
          ],
          currentModelId: "acp/model-a",
          modes: [],
          currentModeId: null,
          configOptions: [],
        },
      }),
    );

    expect(models).toEqual([
      {
        key: "acp:7:acp/model-a",
        providerId: "acp",
        providerName: "本地 ACP",
        modelId: "acp/model-a",
        reasoningLevels: [],
        defaultReasoningLevel: null,
      },
      {
        key: "acp:7:acp/model-b",
        providerId: "acp",
        providerName: "本地 ACP",
        modelId: "acp/model-b",
        reasoningLevels: [],
        defaultReasoningLevel: null,
      },
    ]);
  });

  it("未选择条目或未探测时返回空列表", () => {
    expect(acpDraftModels(undefined)).toEqual([]);
    expect(acpDraftModels(customHarness({}))).toEqual([]);
  });
});

describe("acpDraftReasoningLevels", () => {
  it("取思考类 configOption 的候选值", () => {
    const levels = acpDraftReasoningLevels(
      customHarness({
        capabilities: {
          models: [],
          currentModelId: null,
          modes: [],
          currentModeId: null,
          configOptions: [
            {
              id: "mode",
              name: "Mode",
              category: null,
              optionType: "select",
              currentValue: "ask",
              options: [{ value: "ask", name: "Ask" }],
            },
            {
              id: "reasoning_effort",
              name: "Reasoning",
              category: "thought",
              optionType: "select",
              currentValue: "low",
              options: [
                { value: "low", name: "Low" },
                { value: "high", name: "High" },
              ],
            },
          ],
        },
      }),
    );

    expect(levels).toEqual(["low", "high"]);
  });

  it("未选择条目或没有思考项时返回空列表", () => {
    expect(acpDraftReasoningLevels(undefined)).toEqual([]);
    expect(acpDraftReasoningLevels(customHarness({}))).toEqual([]);
    expect(
      acpDraftReasoningLevels(
        customHarness({
          capabilities: {
            models: [],
            currentModelId: null,
            modes: [],
            currentModeId: null,
            configOptions: [],
          },
        }),
      ),
    ).toEqual([]);
  });
});
