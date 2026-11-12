import { describe, expect, it } from "vitest";
import type { AcpCapabilities } from "@moon/shared";
import { acpReasoningOption, acpSessionModes } from "./acpAgentCapabilities";

const base: AcpCapabilities = {
  models: [],
  currentModelId: null,
  modes: [],
  currentModeId: null,
  configOptions: [],
};

describe("acpSessionModes", () => {
  it("空 capabilities 返回空模式列表", () => {
    expect(acpSessionModes(null)).toEqual([]);
    expect(acpSessionModes(base)).toEqual([]);
  });

  it("返回 agent 宣告的模式列表", () => {
    const capabilities: AcpCapabilities = {
      ...base,
      modes: [
        { id: "plan", name: "Plan" },
        { id: "acceptEdits", name: "Accept Edits" },
      ],
    };
    expect(acpSessionModes(capabilities)).toHaveLength(2);
  });
});

describe("acpReasoningOption", () => {
  it("无 capabilities 或无匹配选项时返回 null", () => {
    expect(acpReasoningOption(null)).toBeNull();
    expect(acpReasoningOption(base)).toBeNull();
  });

  it("按 id 识别思考选项（含 reason）", () => {
    const capabilities: AcpCapabilities = {
      ...base,
      configOptions: [
        {
          id: "model",
          name: "Model",
          category: null,
          optionType: "select",
          currentValue: "m1",
          options: [{ value: "m1", name: "Model 1" }],
        },
        {
          id: "reasoning_effort",
          name: "Reasoning Effort",
          category: null,
          optionType: "select",
          currentValue: "medium",
          options: [
            { value: "low", name: "Low" },
            { value: "high", name: "High" },
          ],
        },
      ],
    };
    const choices = acpReasoningOption(capabilities);
    expect(choices).not.toBeNull();
    expect(choices?.map((choice) => choice.value)).toEqual(["low", "high"]);
  });

  it("按 category 识别思考选项（含 thinking）且跳过非匹配项", () => {
    const capabilities: AcpCapabilities = {
      ...base,
      configOptions: [
        {
          id: "theme",
          name: "Theme",
          category: "display",
          optionType: "select",
          currentValue: "dark",
          options: [{ value: "dark", name: "Dark" }],
        },
        {
          id: "effort",
          name: "Effort",
          category: "Thinking",
          optionType: "select",
          currentValue: "medium",
          options: [{ value: "medium", name: "Medium" }],
        },
      ],
    };
    const choices = acpReasoningOption(capabilities);
    expect(choices?.map((choice) => choice.value)).toEqual(["medium"]);
  });
});
