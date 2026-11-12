import { describe, expect, it } from "vitest";

/**
 * 主题回归守卫：styles.css 是唯一视觉来源。
 *
 * :root 与 :root[data-theme="dark"] 两个设计 token 段允许色值字面量；其余段落
 * 的颜色声明必须引用 var(--…)，防止旧青绿/冷灰蓝调色板以硬编码形式回潮
 * （2026-11-06 复核发现 19 处 rgb()/短 hex 残留，hex 字面量 grep 无法覆盖，
 * 故按家族断言）。
 *
 * 暗色守卫保证 :root[data-theme="dark"] 覆盖亮色块定义的全部颜色 token：
 * 只加亮色 token 而忘记暗色值时，测试直接失败。
 */

/**
 * 用 node:fs 读源文件，而不是 `import css from "./styles.css?raw"`：
 * Vitest 4 未开启 css 处理时 `?raw` 对 .css 返回空字符串（实测 length=0），
 * 三条守卫会退化成对空串断言而空转。
 *
 * 项目未安装 @types/node，故用非字面量 specifier 让 TS 跳过内置模块解析，
 * 返回值的类型在调用处显式声明（只为读一个文件，不为此新增依赖）。
 */
async function readStylesSource(): Promise<string> {
  const specifier: string = "node:fs";
  const fs = (await import(specifier)) as {
    readFileSync: (path: string, encoding: "utf8") => string;
  };
  return fs.readFileSync(siblingPath(import.meta.url, "styles.css"), "utf8");
}

/**
 * 把 import.meta.url 归一化成文件路径。
 * fs 只把 URL 对象当 URL，字符串一律按路径处理，故先剥掉 `file://` 前缀。
 */
function moduleFilePath(moduleUrl: string): string {
  if (!moduleUrl.startsWith("file:")) {
    return moduleUrl;
  }
  const withoutScheme = moduleUrl.replace(/^file:\/{0,3}/, "");
  return /^[a-zA-Z]:\//.test(withoutScheme) ? withoutScheme : `/${withoutScheme}`;
}

/** 由当前模块路径推导同目录文件名。 */
function siblingPath(moduleUrl: string, name: string): string {
  const modulePath = moduleFilePath(moduleUrl);
  return modulePath.slice(0, modulePath.lastIndexOf("/") + 1) + name;
}

const css = await readStylesSource();

const LIGHT_TOKENS = ":root {";
const DARK_TOKENS = ':root[data-theme="dark"] {';

interface BlockRange {
  start: number;
  bodyStart: number;
  end: number;
}

/** 按花括号配对定位一个规则块，返回它在源串中的范围。 */
function tokenBlockRange(source: string, selector: string): BlockRange {
  const start = source.indexOf(selector);
  if (start < 0) {
    throw new Error(`styles.css 缺少 ${selector} 块`);
  }
  const bodyStart = start + selector.length;
  let depth = 1;
  let cursor = bodyStart;
  while (depth > 0 && cursor < source.length) {
    const ch = source[cursor];
    if (ch === "{") depth += 1;
    if (ch === "}") depth -= 1;
    cursor += 1;
  }
  if (depth !== 0) {
    throw new Error(`${selector} 块花括号不配对`);
  }
  return { start, bodyStart, end: cursor };
}

function tokenBlockBody(source: string, selector: string): string {
  const range = tokenBlockRange(source, selector);
  return source.slice(range.bodyStart, range.end - 1);
}

/** 剥离两个 token 块，其余部分都受颜色守卫约束。 */
function cssOutsideTokenBlocks(source: string): string {
  return [LIGHT_TOKENS, DARK_TOKENS]
    .map((selector) => tokenBlockRange(source, selector))
    // 从后往前删，前面块的索引保持有效。
    .sort((a, b) => b.start - a.start)
    .reduce(
      (acc, range) => acc.slice(0, range.start) + acc.slice(range.end),
      source,
    );
}

/** 解析块内声明的 token 名，如 `--color-bg: #fff;` → `--color-bg`。 */
function tokenNames(body: string): string[] {
  return [...body.matchAll(/(--[a-z0-9-]+)\s*:/g)].map((match) => match[1]);
}

const COLOR_TOKEN_PREFIXES = ["--color-", "--status-", "--shadow-", "--bg-glow-"];
const COLOR_TOKEN_NAMES = ["--ink-wash", "--focus-ring"];

function isColorToken(name: string): boolean {
  return (
    COLOR_TOKEN_NAMES.includes(name) ||
    COLOR_TOKEN_PREFIXES.some((prefix) => name.startsWith(prefix))
  );
}

describe("设计 token 守卫", () => {
  it("载入的是真实样式表内容", () => {
    expect(css.length, "styles.css 读取为空，后续守卫会空转").toBeGreaterThan(
      1000,
    );
    expect(css).toContain(LIGHT_TOKENS);
  });

  it("token 段之外不允许 hex 色值字面量", () => {
    const violations = [
      ...cssOutsideTokenBlocks(css).matchAll(/#[0-9a-fA-F]{3,8}\b/g),
    ];
    expect(
      violations.map((match) => match[0]),
      "token 段外发现 hex 色值，请改用 var(--…)",
    ).toEqual([]);
  });

  it("token 段之外不允许 rgb()/rgba()/hsl() 色值字面量", () => {
    const violations = [
      ...cssOutsideTokenBlocks(css).matchAll(/\b(?:rgba?|hsla?)\([^;{}]*\)/g),
    ].filter((match) => !match[0].includes("var("));
    expect(
      violations.map((match) => match[0]),
      "token 段外发现 rgb()/hsl() 色值，请改用 var(--…)",
    ).toEqual([]);
  });

  it("旧青绿/冷灰蓝家族（hex 与 rgb 形式）不得回潮", () => {
    const legacy =
      /#0d9488|#0f766e|#115e59|#ccfbf1|#99f6e4|#ea580c|13, ?148, ?136|234, ?88, ?12|#a78bfa|#059669|#166534|#dcfce7|#b7f7ea|#f59e0b|#d97706|#92400e|#fef3c7|#fde68a|#fca5a5|#991b1b|#7f1d1d|#fff5f5/g;
    expect(css.match(legacy), "发现旧调色板残留").toBeNull();
  });
});

describe("暗色主题 token 覆盖", () => {
  it(':root[data-theme="dark"] 覆盖块存在且非空', () => {
    expect(css).toContain(DARK_TOKENS);
    expect(
      tokenBlockBody(css, DARK_TOKENS).trim().length,
      "暗色 token 块为空",
    ).toBeGreaterThan(0);
  });

  it("亮色块定义的颜色类 token 在暗色块中全部有同名定义", () => {
    const lightTokens = tokenNames(tokenBlockBody(css, LIGHT_TOKENS)).filter(
      isColorToken,
    );
    const darkTokens = new Set(tokenNames(tokenBlockBody(css, DARK_TOKENS)));
    const missing = lightTokens.filter((name) => !darkTokens.has(name));

    expect(
      lightTokens.length,
      "亮色块颜色 token 解析为空，守卫会空转",
    ).toBeGreaterThanOrEqual(40);
    expect(
      missing,
      `暗色块缺少颜色 token，请在 :root[data-theme="dark"] 中补齐：${missing.join(", ")}`,
    ).toEqual([]);
  });
});
