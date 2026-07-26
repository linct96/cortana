import { afterEach, describe, expect, it, vi } from 'vitest';

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.resetModules();
});

describe('listenOAuthProgress', () => {
  it('waits for each web poll and stops after reporting an error', async () => {
    vi.useFakeTimers();
    const storage = new Map<string, string>();
    vi.stubGlobal('window', {
      location: new URL('http://127.0.0.1:11456/'),
      history: { replaceState: vi.fn() },
      setTimeout: globalThis.setTimeout,
      clearTimeout: globalThis.clearTimeout,
    });
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
      removeItem: (key: string) => storage.delete(key),
    });

    let resolvePoll!: (response: Response) => void;
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(jsonResponse({ sequence: 0, pending: true, progress: null }))
      .mockReturnValueOnce(new Promise<Response>((resolve) => (resolvePoll = resolve)))
      .mockRejectedValueOnce(new Error('poll failed'));
    vi.stubGlobal('fetch', fetchMock);

    const { listenOAuthProgress } = await import('./backend');
    const onProgress = vi.fn();
    const onError = vi.fn();
    const stop = await listenOAuthProgress(onProgress, onError);

    await vi.advanceTimersByTimeAsync(1000);
    await vi.advanceTimersByTimeAsync(5000);
    expect(fetchMock).toHaveBeenCalledTimes(2);

    resolvePoll(jsonResponse({ sequence: 1, pending: true, progress: 'waiting' }));
    await vi.advanceTimersByTimeAsync(0);
    expect(onProgress).toHaveBeenCalledWith('waiting');

    await vi.advanceTimersByTimeAsync(1000);
    expect(onError).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(5000);
    expect(fetchMock).toHaveBeenCalledTimes(3);
    stop();
  });
});

function jsonResponse(body: unknown) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}
