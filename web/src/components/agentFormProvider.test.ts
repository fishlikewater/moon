import { describe, expect, it } from "vitest";
import type { ApiProvider } from "@moon/shared";
import {
  findProviderById,
  nextProviderSelection,
  providerModelChoices,
  providerReasoningChoices,
  resolveProviderOptions,
} from "./agentFormProvider";

const PROVIDER = (id: number, enabled: boolean): ApiProvider => ({
  id,
  name: `供应商 ${id}`,
  protocol: "anthropic",
  baseUrl: "https://api.example.com",
  apiKey: "key",
  enabled,
  models: [
    {
      id: "m1",
      name: "M1",
      contextWindow: 65536,
      reasoningLevels: ["standard", "deep"],
      inputTypes: ["text"],
    },
    { id: "m2", name: "M2", contextWindow: null, reasoningLevels: [], inputTypes: [] },
  ],
  agentCount: 0,
  createdAt: 1,
  updatedAt: 1,
});

const ITEMS = [PROVIDER(1, true), PROVIDER(2, false), PROVIDER(3, true)];

describe("resolveProviderOptions", () => {
  it("只保留启用的供应商", () => {
    expect(resolveProviderOptions(ITEMS).map((item) => item.id)).toEqual([1, 3]);
    expect(resolveProviderOptions([])).toEqual([]);
  });
});

describe("findProviderById", () => {
  it("按字符串 id 匹配，空串返回 undefined", () => {
    expect(findProviderById(ITEMS, "1")?.id).toBe(1);
    expect(findProviderById(ITEMS, "2")?.id).toBe(2);
    expect(findProviderById(ITEMS, "")).toBeUndefined();
    expect(findProviderById(ITEMS, "999")).toBeUndefined();
  });
});

describe("providerModelChoices", () => {
  it("返回供应商模型；未绑定返回空", () => {
    expect(providerModelChoices(findProviderById(ITEMS, "1"))).toHaveLength(2);
    expect(providerModelChoices(undefined)).toEqual([]);
  });
});

describe("providerReasoningChoices", () => {
  it("所选模型的思考等级；未选模型或未声明返回空", () => {
    const provider = findProviderById(ITEMS, "1");
    expect(providerReasoningChoices(provider, "m1")).toEqual([
      "standard",
      "deep",
    ]);
    expect(providerReasoningChoices(provider, "m2")).toEqual([]);
    expect(providerReasoningChoices(provider, "")).toEqual([]);
    expect(providerReasoningChoices(undefined, "m1")).toEqual([]);
  });
});

describe("nextProviderSelection", () => {
  it("切换供应商清空模型与思考等级", () => {
    expect(nextProviderSelection("3")).toEqual({
      apiProviderId: "3",
      model: "",
      reasoningLevel: "",
    });
  });
});
