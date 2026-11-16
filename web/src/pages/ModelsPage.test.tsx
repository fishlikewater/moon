import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";
import HarnessesPage from "./HarnessesPage";
import ModelsPage from "./ModelsPage";

function render(page: React.ReactNode): string {
  return renderToStaticMarkup(<MemoryRouter>{page}</MemoryRouter>);
}

describe("模型菜单页面", () => {
  it("/models 独立渲染模型供应商注册表", () => {
    const html = render(<ModelsPage />);

    expect(html).toContain("<h1>模型</h1>");
    expect(html).toContain("模型供应商");
    expect(html).toContain("新建供应商");
  });

  it("Harness 页不再渲染模型供应商区块", () => {
    const html = render(<HarnessesPage />);

    expect(html).toContain("<h1>Harness</h1>");
    expect(html).not.toContain("模型供应商");
    expect(html).not.toContain("api-providers");
  });
});
