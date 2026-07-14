import { invoke } from '@tauri-apps/api/core';
import CodeMirror, { EditorView } from '@uiw/react-codemirror';
import { StreamLanguage } from '@codemirror/language';
import { linter, lintGutter } from '@codemirror/lint';
import { toml } from '@codemirror/legacy-modes/mode/toml';
import { LoaderCircle, Save, WandSparkles } from 'lucide-react';
import { type FormEvent, useEffect, useState } from 'react';
import { toast } from 'sonner';
import { PageHeader, PageShell } from './components/page-shell';
import { Button } from './components/ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from './components/ui/tooltip';
import { appError } from './utils';

type CodexConfigFile = {
  path: string;
  content: string;
};

type CodexConfigDiagnostic = {
  from: number;
  to: number;
  message: string;
};

const configEditorExtensions = [
  StreamLanguage.define(toml),
  lintGutter(),
  linter(
    (view) =>
      invoke<CodexConfigDiagnostic[]>('validate_codex_config', {
        content: view.state.doc.toString(),
      }).then((diagnostics) =>
        diagnostics.map((diagnostic) => ({ ...diagnostic, severity: 'error' as const })),
      ),
    { delay: 300 },
  ),
  EditorView.contentAttributes.of({
    'aria-label': 'config.toml 内容',
    spellcheck: 'false',
  }),
  EditorView.theme({
    '&': {
      backgroundColor: 'transparent',
      color: 'var(--foreground)',
      fontSize: '12px',
    },
    '&.cm-focused': { outline: 'none' },
    '.cm-content': {
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace',
      padding: '8px 0',
    },
    '.cm-gutters': {
      backgroundColor: 'var(--muted)',
      borderRight: '1px solid var(--border)',
      color: 'var(--muted-foreground)',
    },
    '.cm-activeLine, .cm-activeLineGutter': {
      backgroundColor: 'var(--accent)',
    },
  }),
];

export default function ConfigPage() {
  const [config, setConfig] = useState<CodexConfigFile | null>(null);
  const [content, setContent] = useState('');
  const [savedContent, setSavedContent] = useState('');
  const [busy, setBusy] = useState(true);
  const [formatting, setFormatting] = useState(false);

  useEffect(() => {
    invoke<CodexConfigFile>('get_codex_config')
      .then((next) => {
        setConfig(next);
        setContent(next.content);
        setSavedContent(next.content);
      })
      .catch((error) => toast.error(appError(error)))
      .finally(() => setBusy(false));
  }, []);

  async function saveConfig(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    try {
      await invoke('save_codex_config', { content });
      setSavedContent(content);
      toast.success('Codex 配置已保存。');
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setBusy(false);
    }
  }

  async function formatConfig() {
    setFormatting(true);
    try {
      setContent(await invoke<string>('format_codex_config', { content }));
      toast.success('TOML 已格式化。');
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setFormatting(false);
    }
  }

  return (
    <PageShell className="flex flex-col">
      <PageHeader title="Codex 配置" />

      <form
        className="flex min-h-0 w-full flex-1 flex-col gap-3 px-4 pb-6 sm:px-8 sm:pb-7 lg:px-12"
        onSubmit={saveConfig}
      >
        <div className="flex min-w-0 items-center justify-between gap-3">
          <code className="truncate text-xs text-muted-foreground" title={config?.path}>
            {config?.path ?? '正在读取 config.toml'}
          </code>
          <div className="flex shrink-0 items-center gap-2">
            <Tooltip>
              <TooltipTrigger render={<span className="inline-flex" />}>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  onClick={() => void formatConfig()}
                  disabled={busy || formatting || !config}
                >
                  {formatting ? <LoaderCircle className="animate-spin" /> : <WandSparkles />}
                  <span className="sr-only">格式化 TOML</span>
                </Button>
              </TooltipTrigger>
              <TooltipContent>格式化 TOML</TooltipContent>
            </Tooltip>
            <Button
              type="submit"
              disabled={busy || formatting || !config || content === savedContent}
            >
              {busy ? <LoaderCircle className="animate-spin" /> : <Save />}
              保存
            </Button>
          </div>
        </div>
        <CodeMirror
          className="min-h-0 flex-1 overflow-hidden rounded-lg border border-input focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/50"
          height="100%"
          value={content}
          onChange={setContent}
          extensions={configEditorExtensions}
          editable={!busy}
          readOnly={busy}
          theme="none"
        />
      </form>
    </PageShell>
  );
}
