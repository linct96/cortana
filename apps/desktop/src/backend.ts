import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

const DEFAULT_WEB_PORT = 11456;
const TOKEN_KEY = 'cortana.webToken';
const PORT_KEY = 'cortana.webPort';

export const isTauri = '__TAURI_INTERNALS__' in window;

if (!isTauri) {
  localStorage.removeItem(TOKEN_KEY);
  const url = new URL(window.location.href);
  const bootstrap = new URLSearchParams(url.hash.slice(1));
  const token = bootstrap.get('token');
  const port = Number(bootstrap.get('port'));
  if (token) sessionStorage.setItem(TOKEN_KEY, token);
  if (Number.isInteger(port) && port >= 1024 && port <= 65535) {
    localStorage.setItem(PORT_KEY, String(port));
  }
  if (token || bootstrap.has('port')) {
    window.history.replaceState(null, '', `${url.pathname}${url.search}#/`);
  }
}

export async function invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  if (isTauri) return tauriInvoke<T>('invoke_local', { command, args });

  let response: Response;
  try {
    const token = sessionStorage.getItem(TOKEN_KEY);
    response = await fetch(`${webApiOrigin()}/api/invoke`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
      },
      body: JSON.stringify({ command, args }),
    });
  } catch {
    throw new Error('Cortana Web 访问未启用或本机服务未运行。');
  }
  if (!response.ok) {
    if (response.status === 401) sessionStorage.removeItem(TOKEN_KEY);
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

function webApiOrigin() {
  if (!import.meta.env.DEV) return window.location.origin;
  const savedPort = Number(localStorage.getItem(PORT_KEY));
  const port = Number.isInteger(savedPort) && savedPort >= 1024 ? savedPort : DEFAULT_WEB_PORT;
  return `http://127.0.0.1:${port}`;
}
