import { afterEach, describe, expect, it, vi } from 'vitest';

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.resetModules();
});

describe('web transport', () => {
  it('bootstraps an ephemeral token from the fragment and clears it after 401', async () => {
    const local = new Map<string, string>();
    local.set('cortana.webToken', 'legacy-secret');
    const session = new Map<string, string>();
    const replaceState = vi.fn();
    vi.stubGlobal('window', {
      location: new URL('http://127.0.0.1:5173/#token=secret&port=11457'),
      history: { replaceState },
    });
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => local.get(key) ?? null,
      setItem: (key: string, value: string) => local.set(key, value),
      removeItem: (key: string) => local.delete(key),
    });
    vi.stubGlobal('sessionStorage', {
      getItem: (key: string) => session.get(key) ?? null,
      setItem: (key: string, value: string) => session.set(key, value),
      removeItem: (key: string) => session.delete(key),
    });
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(jsonResponse({ ready: true }))
      .mockResolvedValueOnce(new Response('unauthorized', { status: 401 }));
    vi.stubGlobal('fetch', fetchMock);

    const { invoke } = await import('./backend');
    await invoke('get_app_status');

    expect(session.get('cortana.webToken')).toBe('secret');
    expect(local.has('cortana.webToken')).toBe(false);
    expect(local.get('cortana.webPort')).toBe('11457');
    expect(replaceState).toHaveBeenCalledWith(null, '', '/#/');
    expect(fetchMock).toHaveBeenCalledWith(
      'http://127.0.0.1:11457/api/invoke',
      expect.objectContaining({
        headers: expect.objectContaining({ Authorization: 'Bearer secret' }),
      }),
    );

    await expect(invoke('get_app_status')).rejects.toThrow('unauthorized');
    expect(session.has('cortana.webToken')).toBe(false);
  });
});

function jsonResponse(body: unknown) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}
