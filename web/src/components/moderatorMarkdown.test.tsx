import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import MarkdownContent from "./MarkdownContent";
import {
  CONCLUSION_WALL_MIN,
  formatDiscussionConclusion,
} from "./discussionView";
import { formatModeratorTranscript } from "./moderatorProtocol";

const markdown = [
  "## 结论",
  "",
  "- 采用方案 A",
  "- 先做小流量验证",
  "",
  "## 下一步",
  "",
  "- 补齐监控与回滚",
].join("\n");

describe("主持人 Markdown 呈现", () => {
  it("主持人消息渲染出二级标题与列表", () => {
    const html = renderToStaticMarkup(
      <article className="msg msg-agent msg-final msg-moderator">
        <MarkdownContent text={formatModeratorTranscript(markdown, undefined, true)} />
      </article>,
    );
    expect(html).toContain('class="msg msg-agent msg-final msg-moderator"');
    expect(html).toContain("<h2>结论</h2>");
    expect(html).toContain("<h2>下一步</h2>");
    expect(html).toContain("<ul>");
    expect(html).toContain("<li>采用方案 A</li>");
  });

  it("旧协议块折叠后仍保留 Markdown 层级", () => {
    const raw = [
      markdown,
      "",
      "```json",
      JSON.stringify({
        type: "assign",
        assignments: [{ agentId: 1, instruction: "验证回滚" }],
      }),
      "```",
    ].join("\n");
    const display = formatModeratorTranscript(raw, () => "成员");
    expect(display).not.toContain('"type":"assign"');
    const html = renderToStaticMarkup(<MarkdownContent text={display} />);
    expect(html).toContain("<h2>结论</h2>");
    expect(html).toContain("<li>采用方案 A</li>");
  });

  it("讨论裁决的结构化 Markdown 原样保留并渲染", () => {
    const display = formatDiscussionConclusion(`讨论结论：\n\n${markdown}`, "judge");
    expect(display).toContain("## 结论");
    const html = renderToStaticMarkup(<MarkdownContent text={display} />);
    expect(html).toContain("<h2>结论</h2>");
    expect(html).toContain("<li>采用方案 A</li>");
  });

  it("长结构化裁决不被旧文字墙兜底改写", () => {
    const longMarkdown = [
      "## 结论",
      "",
      "采用方案 A。先做小流量验证；接口与页面已完成，搜索与筛选功能均已联调通过。",
      "",
      "## 进展",
      "",
      "- 搜索接口已交付。空态与错误提示已补齐；监控已接入，告警阈值沿用既有配置。",
      "- 回滚脚本与灰度开关已就绪。数据迁移脚本在预发环境执行完毕；索引重建耗时符合预期。",
      "",
      "## 风险与待确认",
      "",
      "- 灰度范围待确认；回滚脚本尚未演练，需要在本周内安排一次全链路演练。",
      "- 观测指标口径待对齐。搜索延迟与零结果率的基线仍在收集；需要产品确认可接受的波动范围。",
      "",
      "## 下一步",
      "",
      "- 确认灰度名单。与运营对齐放量节奏；明天上午给出首轮名单。",
      "- 运行回滚演练并记录结果；补充灰度观测指标，并在演练后更新发布检查单。",
    ].join("\n");
    // 自证 fixture 越过旧文字墙门槛，负向证明才有意义。
    expect(longMarkdown.length).toBeGreaterThan(CONCLUSION_WALL_MIN);
    expect(longMarkdown).not.toContain("\n\n\n");
    const content = `讨论结论：\n\n${longMarkdown}`;
    const display = formatDiscussionConclusion(content, "judge");
    expect(display).toBe(content);
    expect(display).toContain("\n");
    const html = renderToStaticMarkup(<MarkdownContent text={display} />);
    expect(html).toContain("<h2>结论</h2>");
    expect(html).toContain("<li>搜索接口已交付。空态与错误提示已补齐；监控已接入，告警阈值沿用既有配置。</li>");
  });
});
