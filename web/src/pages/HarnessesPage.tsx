import { useCallback, useState } from "react";
import type { CustomHarness } from "@moon/shared";
import {
  AcpOnboardingGuide,
  shouldShowOnboarding,
} from "../components/AcpOnboardingGuide";
import { AcpHarnessRegistry, acpUsableFromItems } from "../components/AcpHarnessRegistry";
import { HarnessOverview } from "../components/HarnessOverview";
import { PageHeader } from "../components/layout/PageHeader";
import { CATALOG_READABLE_KINDS } from "../harnessCatalog";
import { useHarnessCatalogs } from "../useHarnessCatalogs";

/** Harness 页面：内置 harness 概览、新手引导与自定义 ACP agent 注册表。 */
export default function HarnessesPage() {
  const [acpCount, setAcpCount] = useState(0);
  /** 任一条目「当前可用」（最近一次失败优先于旧成功快照）才算有可用接入。 */
  const [acpUsable, setAcpUsable] = useState(false);
  const [registryLoaded, setRegistryLoaded] = useState(false);
  const catalogs = useHarnessCatalogs(CATALOG_READABLE_KINDS);

  const internalUsable = CATALOG_READABLE_KINDS.some(
    (kind) =>
      catalogs[kind]?.status === "ready" && catalogs[kind].catalog.available,
  );
  const catalogsSettled = CATALOG_READABLE_KINDS.every(
    (kind) => catalogs[kind]?.status !== "loading",
  );

  // 稳定回调：注册表组件用它回传条目数，避免 effect 因新函数身份重复加载。
  const handleRegistryChanged = useCallback((items: CustomHarness[]) => {
    setAcpCount(items.length);
    setAcpUsable(acpUsableFromItems(items));
    setRegistryLoaded(true);
  }, []);

  return (
    <div className="page harness-page">
      <PageHeader
        title="Harness"
        subtitle="接入本机 AI agent 的运行方式：内置适配器与自定义 ACP 注册表。"
      />
      <AcpOnboardingGuide
        visible={shouldShowOnboarding({
          settled: registryLoaded && catalogsSettled,
          internalUsable,
          acpUsable,
        })}
      />
      <HarnessOverview states={catalogs} acpCount={acpCount} />
      <AcpHarnessRegistry onChanged={handleRegistryChanged} />
    </div>
  );
}
