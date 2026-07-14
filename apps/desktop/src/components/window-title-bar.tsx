import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useState } from 'react';
import styles from './window-title-bar.module.css';

const appWindow = '__TAURI_INTERNALS__' in window ? getCurrentWindow() : null;

export function WindowTitleBar() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!appWindow) return;
    const syncMaximized = () => void appWindow.isMaximized().then(setMaximized);
    syncMaximized();
    const unlisten = appWindow.onResized(syncMaximized);
    return () => void unlisten.then((stop) => stop());
  }, []);

  const toggleMaximize = async () => {
    if (!appWindow) return;
    await appWindow.toggleMaximize();
    setMaximized(await appWindow.isMaximized());
  };

  return (
    <header data-tauri-drag-region className={styles.root} aria-label="窗口标题栏">
      <div
        data-tauri-drag-region
        className={styles.dragRegion}
        onDoubleClick={() => void toggleMaximize()}
      />
      <CaptionButton label="最小化" icon="minimize" onClick={() => appWindow?.minimize()} />
      <CaptionButton
        label={maximized ? '还原' : '最大化'}
        icon={maximized ? 'restore' : 'maximize'}
        onClick={toggleMaximize}
      />
      <CaptionButton label="关闭" icon="close" close onClick={() => appWindow?.close()} />
    </header>
  );
}

function CaptionButton({
  label,
  icon,
  close = false,
  onClick,
}: {
  label: string;
  icon: CaptionIcon;
  close?: boolean;
  onClick: () => void | Promise<void>;
}) {
  return (
    <button
      className={close ? `${styles.button} ${styles.closeButton}` : styles.button}
      type="button"
      aria-label={label}
      title={label}
      onClick={() => void onClick()}
    >
      <CaptionIconGlyph icon={icon} />
    </button>
  );
}

type CaptionIcon = 'minimize' | 'maximize' | 'restore' | 'close';

function CaptionIconGlyph({ icon }: { icon: CaptionIcon }) {
  return (
    <svg className={styles.icon} viewBox="0 0 32 32" aria-hidden="true">
      {icon === 'minimize' && <path d="M4 16h24" />}
      {icon === 'maximize' && <rect x="5" y="5" width="22" height="22" rx="3" />}
      {icon === 'restore' && (
        <>
          <path d="M11 5h16v16" />
          <rect x="5" y="11" width="16" height="16" rx="3" />
        </>
      )}
      {icon === 'close' && (
        <>
          <path d="M5 5l22 22" />
          <path d="M27 5L5 27" />
        </>
      )}
    </svg>
  );
}
