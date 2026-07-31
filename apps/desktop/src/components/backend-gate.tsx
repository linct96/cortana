import { LoaderCircle, RefreshCw } from 'lucide-react';
import { type ReactNode, useCallback, useEffect, useState } from 'react';
import { invoke, isTauri } from '../backend';
import { Alert, AlertDescription, AlertTitle } from './ui/alert';
import { Button } from './ui/button';

export function BackendGate({ children }: { children: ReactNode }) {
  const [state, setState] = useState<'loading' | 'ready' | string>(isTauri ? 'ready' : 'loading');

  const connect = useCallback(async () => {
    setState('loading');
    try {
      await invoke('get_app_status', { product: 'codex' });
      setState('ready');
    } catch (error) {
      setState(error instanceof Error ? error.message : String(error));
    }
  }, []);

  useEffect(() => {
    if (!isTauri) void connect();
  }, [connect]);

  if (state === 'ready') return children;
  if (state === 'loading') {
    return (
      <div className="flex h-screen items-center justify-center text-muted-foreground">
        <LoaderCircle className="size-5 animate-spin" aria-label="正在连接本机服务" />
      </div>
    );
  }
  return (
    <main className="flex h-screen items-center justify-center p-6">
      <Alert className="max-w-md" variant="destructive">
        <AlertTitle>无法连接 Cortana</AlertTitle>
        <AlertDescription>{state}</AlertDescription>
        <Button className="mt-4" type="button" variant="outline" onClick={() => void connect()}>
          <RefreshCw data-icon="inline-start" />
          重试
        </Button>
      </Alert>
    </main>
  );
}
