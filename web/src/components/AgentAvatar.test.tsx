import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { AgentAvatar } from "./AgentAvatar";

describe("AgentAvatar", () => {
  it("按 id 输出色相类与首字，不写内联颜色", () => {
    const html = renderToStaticMarkup(<AgentAvatar id={3} name="策划" />);

    expect(html).toContain("avatar-tone-2");
    expect(html).toContain(">策<");
    expect(html).not.toContain("style=");
  });

  it("尺寸类与空白名称回退", () => {
    const html = renderToStaticMarkup(
      <AgentAvatar id={1} name="   " size="sm" />,
    );

    expect(html).toContain("avatar-sm");
    expect(html).toContain(">?<");
  });
});
