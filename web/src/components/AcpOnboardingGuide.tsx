/**
 * 新手引导：没有任何可用接入时显示三步上手路径。
 *
 * 显示条件由 `shouldShowOnboarding` 纯函数决定：内置 harness 任一模型
 * 目录可用、或任一 ACP 条目探测成功、或页面数据尚未就绪时都不显示，
 * 避免加载过程中闪烁。
 */

/** 引导可见性输入；`settled` 表示两份数据（内置目录 + 注册表）都已加载。 */
export type OnboardingVisibilityInput = {
  settled: boolean;
  internalUsable: boolean;
  acpUsable: boolean;
};

/** 纯函数：是否显示新手引导。 */
export function shouldShowOnboarding(input: OnboardingVisibilityInput): boolean {
  return (
    input.settled && !input.internalUsable && !input.acpUsable
  );
}

export function AcpOnboardingGuide({ visible }: { visible: boolean }) {
  if (!visible) {
    return null;
  }
  return (
    <section className="card acp-onboarding" aria-label="接入引导">
      <h2 className="card-title">还没有可用的 agent 接入</h2>
      <p className="field-hint">
        按下面三步操作，让本地 agent 在 Moon 的房间里开工。
      </p>
      <ol className="acp-onboarding-steps">
        <li>
          安装{" "}
          <a href="https://nodejs.org/" target="_blank" rel="noreferrer">
            Node.js
          </a>
          （自定义 ACP agent 依赖它运行），安装完成后重启 Moon。
        </li>
        <li>
          在下方
          <a href="#acp-registry">「自定义 ACP agent」注册表</a>
          点「新建 ACP agent」，从模板选择一个 agent。
        </li>
        <li>
          点「检测能力」确认可用；从未检测的条目会自动检测，失败会显示具体原因。
        </li>
      </ol>
    </section>
  );
}
