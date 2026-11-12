import { useCallback, useState } from "react";
import type { CustomHarness } from "@moon/shared";
import { AcpHarnessRegistry } from "../components/AcpHarnessRegistry";
import { HarnessOverview } from "../components/HarnessOverview";
import { PageHeader } from "../components/layout/PageHeader";
import { CATALOG_READABLE_KINDS } from "../harnessCatalog";
import { useHarnessCatalogs } from "../useHarnessCatalogs";

/** Harness 页面：内置 harness 概览与自定义 ACP agent 注册表。 */
export default function HarnessesPage() {
  const [acpCount, setAcpCount] = useState(0);
  const catalogs = useHarnessCatalogs(CATALOG_READABLE_KINDS);

  // 稳定回调：注册表组件用它回传条目数，避免 effect 因新函数身份重复加载。
  const handleRegistryChanged = useCallback((items: CustomHarness[]) => {
    setAcpCount(items.length);
  }, []);

  return (
    <div className="page harness-page">
      <PageHeader
        title="Harness"
        subtitle="接入本机 AI agent 的运行方式：内置适配器与自定义 ACP 注册表。"
      />
      <HarnessOverview states={catalogs} acpCount={acpCount} />
      <AcpHarnessRegistry onChanged={handleRegistryChanged} />
    </div>
  );
}
