import { invoke } from '../../backend';
import CodeMirror, { EditorView } from '@uiw/react-codemirror';
import { StreamLanguage } from '@codemirror/language';
import { linter, lintGutter } from '@codemirror/lint';
import { json } from '@codemirror/legacy-modes/mode/javascript';
import { toml } from '@codemirror/legacy-modes/mode/toml';
import { LoaderCircle, Save, WandSparkles } from 'lucide-react';
import { type FormEvent, useEffect, useMemo, useState } from 'react';
import { toast } from 'sonner';
import { type AccountProduct, useAppShell } from '../../components/app-shell-context';
import { PageHeader, PageShell } from '../../components/page-shell';
import { Button } from '../../components/ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../components/ui/tooltip';
import { appError } from '../../utils';

type ConfigMetadata = {
  title: string;
  filename: string;
  formatName: 'TOML' | 'JSON';
  language: typeof toml | typeof json;
};

const configMetadata: Record<AccountProduct, ConfigMetadata> = {
  codex: {
    title: 'Codex 配置',
    filename: 'config.toml',
    formatName: 'TOML',
    language: toml,
  },
  claude: {
    title: 'Claude 配置',
    filename: 'settings.json',
    formatName: 'JSON',
    language: json,
  },
  antigravity: {
    title: 'Antigravity 配置',
    filename: 'settings.json',
    formatName: 'JSON',
    language: json,
  },
  grok: {
    title: 'Grok 配置',
    filename: 'config.toml',
    formatName: 'TOML',
    language: toml,
  },
};

type ConfigFile = {
  path: string;
  content: string;
};

type ConfigDiagnostic = {
  from: number;
  to: number;
  message: string;
};

const editorTheme = EditorView.theme({
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
  '.cm-activeLineGutter': {
    backgroundColor: 'var(--accent)',
  },
});

function configEditorExtensions(product: AccountProduct) {
  const { filename, language } = configMetadata[product];
  return [
    StreamLanguage.define(language),
    lintGutter(),
    linter(
      (view) =>
        invoke<ConfigDiagnostic[]>(`validate_${product}_config`, {
          content: view.state.doc.toString(),
        }).then((diagnostics) =>
          diagnostics.map((diagnostic) => ({ ...diagnostic, severity: 'error' as const })),
        ),
      { delay: 300 },
    ),
    EditorView.contentAttributes.of({
      'aria-label': `${filename} 内容`,
      spellcheck: 'false',
    }),
    editorTheme,
  ];
}

export default function ConfigPage() {
  const { activeProduct } = useAppShell();
  return (
    <PageShell className="flex flex-col">
      <PageHeader title={configMetadata[activeProduct].title} />
      <ConfigContent key={activeProduct} product={activeProduct} />
    </PageShell>
  );
}

function ConfigContent({ product }: { product: AccountProduct }) {
  const [config, setConfig] = useState<ConfigFile | null>(null);
  const [content, setContent] = useState('');
  const [savedContent, setSavedContent] = useState('');
  const [busy, setBusy] = useState(true);
  const [formatting, setFormatting] = useState(false);
  const extensions = useMemo(() => configEditorExtensions(product), [product]);
  const { title, formatName, filename } = configMetadata[product];

  useEffect(() => {
    let ignore = false;
    invoke<ConfigFile>(`get_${product}_config`)
      .then((next) => {
        if (ignore) return;
        setConfig(next);
        setContent(next.content);
        setSavedContent(next.content);
      })
      .catch((error) => {
        if (!ignore) toast.error(appError(error));
      })
      .finally(() => {
        if (!ignore) setBusy(false);
      });
    return () => {
      ignore = true;
    };
  }, [product]);

  async function saveConfig(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    try {
      await invoke(`save_${product}_config`, { content });
      setSavedContent(content);
      toast.success(`${title}已保存。`);
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setBusy(false);
    }
  }

  async function formatConfig() {
    setFormatting(true);
    try {
      setContent(await invoke<string>(`format_${product}_config`, { content }));
      toast.success(`${formatName} 已格式化。`);
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setFormatting(false);
    }
  }

  return (
    <>
      <form
        className="flex min-h-0 w-full flex-1 flex-col gap-3 px-4 pb-6 sm:px-8 sm:pb-7 lg:px-12"
        onSubmit={saveConfig}
      >
        <div className="flex min-w-0 items-center justify-between gap-3">
          <code className="truncate text-xs text-muted-foreground" title={config?.path}>
            {config?.path ?? `正在读取 ${filename}`}
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
                  <span className="sr-only">格式化 {formatName}</span>
                </Button>
              </TooltipTrigger>
              <TooltipContent>格式化 {formatName}</TooltipContent>
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
          extensions={extensions}
          editable={!busy}
          readOnly={busy}
          theme="none"
        />
      </form>
    </>
  );
}
