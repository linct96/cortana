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
      <CaptionButton label="最小化" glyph="\uE921" onClick={() => appWindow?.minimize()} />
      <CaptionButton
        label={maximized ? '还原' : '最大化'}
        glyph={maximized ? '\uE923' : '\uE922'}
        onClick={toggleMaximize}
      />
      <CaptionButton label="关闭" glyph="\uE8BB" close onClick={() => appWindow?.close()} />
    </header>
  );
}

function CaptionButton({
  label,
  glyph,
  close = false,
  onClick,
}: {
  label: string;
  glyph: string;
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
      <span className={styles.glyph} aria-hidden="true">
        {glyph}
      </span>
    </button>
  );
}
