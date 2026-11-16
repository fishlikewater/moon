import { describe, expect, it } from "vitest";
import type { ApiProvider } from "@moon/shared";
import {
  isModelRowEmpty,
  parseCommaList,
  providerFormFromItem,
  serializeProviderForm,
  validateProviderForm,
} from "./providerForm";

const ITEM: ApiProvider = {
  id: 7,
  name: "DeepSeek",
  protocol: "anthropic",
  baseUrl: "https://api.deepseek.com/anthropic",
  apiKey: "sk-test",
  enabled: false,
  models: [
    {
      id: "deepseek-chat",
      name: "DeepSeek Chat",
      contextWindow: 65536,
      reasoningLevels: ["standard", "deep"],
      inputTypes: ["text", "image"],
    },
  ],
  agentCount: 2,
  createdAt: 1,
  updatedAt: 2,
};

describe("providerFormFromItem", () => {
  it("新建返回空表单，编辑回显条目", () => {
    const blank = providerFormFromItem(null);
    expect(blank.editingId).toBeNull();
    expect(blank.enabled).toBe(true);
    expect(blank.models).toHaveLength(1);

    const form = providerFormFromItem(ITEM);
    expect(form.editingId).toBe(7);
    expect(form.protocol).toBe("anthropic");
    expect(form.baseUrl).toBe("https://api.deepseek.com/anthropic");
    expect(form.enabled).toBe(false);
    expect(form.models[0].contextWindowText).toBe("65536");
    expect(form.models[0].reasoningText).toBe("standard, deep");
    expect(form.models[0].inputTypes).toEqual(["text", "image"]);
  });

  it("无模型的条目回退为一个空行", () => {
    const form = providerFormFromItem({ ...ITEM, models: [] });
    expect(form.models).toHaveLength(1);
    expect(isModelRowEmpty(form.models[0])).toBe(true);
  });
});

describe("serializeProviderForm 与表单往返一致", () => {
  it("编辑回显后序列化还原条目内容", () => {
    const form = providerFormFromItem(ITEM);
    const payload = serializeProviderForm(form);
    expect(payload.name).toBe("DeepSeek");
    expect(payload.protocol).toBe("anthropic");
    expect(payload.baseUrl).toBe(ITEM.baseUrl);
    expect(payload.apiKey).toBe("sk-test");
    expect(payload.enabled).toBe(false);
    expect(payload.models).toEqual(ITEM.models);
  });

  it("剔除全空行、缺省 name 取 id、空窗口为 null", () => {
    const form = providerFormFromItem(null);
    form.models = [
      { id: "", name: "", contextWindowText: "", reasoningText: "", inputTypes: [] },
      { id: "m1", name: "", contextWindowText: "", reasoningText: "", inputTypes: [] },
      { id: "m2", name: "M2", contextWindowText: "", reasoningText: "", inputTypes: [] },
    ];
    const payload = serializeProviderForm(form);
    expect(payload.models).toHaveLength(2);
    expect(payload.models[0]).toEqual({
      id: "m1",
      name: "m1",
      contextWindow: null,
      reasoningLevels: [],
      inputTypes: [],
    });
  });
});

describe("validateProviderForm", () => {
  it("空名、坏 scheme、非正整数窗口与重复 id 都报错", () => {
    const form = providerFormFromItem(null);
    form.name = "  ";
    form.baseUrl = "api.example.com";
    form.models = [
      { id: "m1", name: "", contextWindowText: "0", reasoningText: "", inputTypes: [] },
      { id: "m1", name: "", contextWindowText: "abc", reasoningText: "", inputTypes: [] },
    ];
    const errors = validateProviderForm(form);
    expect(errors).toContain("名称不能为空");
    expect(errors).toContain("Base URL 必须以 http:// 或 https:// 开头");
    expect(errors.some((error) => error.includes("上下文大小"))).toBe(true);
    expect(errors.some((error) => error.includes("重复"))).toBe(true);
  });

  it("合法表单无错误；全空行不触发 id 校验", () => {
    const form = providerFormFromItem(ITEM);
    form.models.push({
      id: "",
      name: "",
      contextWindowText: "",
      reasoningText: "",
      inputTypes: [],
    });
    expect(validateProviderForm(form)).toEqual([]);
  });
});

describe("parseCommaList", () => {
  it("支持中英文逗号并剔除空项", () => {
    expect(parseCommaList("a, b，c ,, d")).toEqual(["a", "b", "c", "d"]);
    expect(parseCommaList("")).toEqual([]);
  });
});
