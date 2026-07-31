import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export const isTauri = '__TAURI_INTERNALS__' in window;

export async function invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  if (isTauri) return tauriInvoke<T>('invoke_local', { command, args });

  let response: Response;
  try {
    response = await fetch('/api/invoke', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ command, args }),
    });
  } catch {
    throw new Error('Cortana Web 访问未启用或本机服务未运行。');
  }
  if (!response.ok) {
    throw new Error((await response.text()) || `本机服务请求失败（HTTP ${response.status}）。`);
  }
  return response.json() as Promise<T>;
}

type OAuthProgressSnapshot<T> = {
  sequence: number;
  pending: boolean;
  progress: T | null;
};

export async function listenOAuthProgress<T>(
  handler: (progress: T) => void,
  onError: (error: unknown) => void,
): Promise<() => void> {
  if (isTauri) {
    return listen<T>('oauth-progress', ({ payload }) => handler(payload));
  }

  let active = true;
  let cursor = 0;
  const initial = await invoke<OAuthProgressSnapshot<T>>('get_oauth_progress');
  cursor = initial.sequence;
  if (initial.pending && initial.progress) handler(initial.progress);
  let timer: number | undefined;
  async function poll() {
    try {
      const snapshot = await invoke<OAuthProgressSnapshot<T>>('get_oauth_progress');
      if (!active) return;
      if (snapshot.sequence > cursor) {
        cursor = snapshot.sequence;
        if (snapshot.progress) handler(snapshot.progress);
      }
    } catch (error) {
      if (!active) return;
      active = false;
      onError(error);
      return;
    }
    if (active) timer = window.setTimeout(() => void poll(), 1000);
  }
  timer = window.setTimeout(() => void poll(), 1000);
  return () => {
    active = false;
    if (timer !== undefined) window.clearTimeout(timer);
  };
}
