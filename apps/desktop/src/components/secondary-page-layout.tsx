import { Link } from '@tanstack/react-router';
import { ArrowLeft } from 'lucide-react';
import type { ReactNode } from 'react';
import { AppContent } from './app-shell';
import { type MainPath, useAppShell } from './app-shell-context';
import { AppSidebar } from './app-sidebar';

export function SecondaryPageLayout({
  title,
  backTo,
  navigation,
  children,
}: {
  title: string;
  backTo: MainPath;
  navigation: ReactNode;
  children: ReactNode;
}) {
  const { topPadding } = useAppShell();

  return (
    <>
      <AppSidebar
        className={topPadding}
        header={
          <Link
            to={backTo}
            className="flex h-9 items-center gap-2 rounded-md px-2 outline-none transition-colors hover:bg-accent focus-visible:ring-3 focus-visible:ring-ring/50"
          >
            <ArrowLeft className="size-4 shrink-0" />
            <strong className="truncate text-base font-semibold">{title}</strong>
          </Link>
        }
        navigation={navigation}
        navigationLabel={`${title}导航`}
      />
      <AppContent>{children}</AppContent>
    </>
  );
}
