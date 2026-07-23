import { Outlet } from '@tanstack/react-router';
import { CreditCard, Info, SlidersHorizontal } from 'lucide-react';
import { useAppShell } from '../../components/app-shell-context';
import { SidebarNavItem } from '../../components/app-sidebar';
import { SecondaryPageLayout } from '../../components/secondary-page-layout';

export default function SettingsLayout() {
  const { activeProduct, previousMainPath } = useAppShell();

  return (
    <SecondaryPageLayout
      title="设置"
      backTo={activeProduct === 'codex' ? previousMainPath : '/accounts'}
      navigation={
        <>
          <SidebarNavItem to="/settings/general" label="常规" icon={SlidersHorizontal} />
          <SidebarNavItem to="/settings/billing" label="计费" icon={CreditCard} />
          <SidebarNavItem to="/settings/about" label="关于" icon={Info} />
        </>
      }
    >
      <Outlet />
    </SecondaryPageLayout>
  );
}
