import { afterEach, describe, expect, it, vi } from 'vitest';

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.resetModules();
});

describe('web transport', () => {
  it('invokes through the same-origin API path', async () => {
    vi.stubGlobal('window', {});
    const fetchMock = vi.fn().mockResolvedValueOnce(jsonResponse({ ready: true }));
    vi.stubGlobal('fetch', fetchMock);

    const { invoke } = await import('./backend');
    await invoke('get_app_status');

    expect(fetchMock).toHaveBeenCalledWith(
      '/api/invoke',
      expect.objectContaining({
        headers: { 'Content-Type': 'application/json' },
      }),
    );
  });
});

function jsonResponse(body: unknown) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}
