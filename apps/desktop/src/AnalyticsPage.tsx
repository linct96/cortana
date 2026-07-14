import { invoke } from '@tauri-apps/api/core';
import { LoaderCircle, RefreshCw, TriangleAlert } from 'lucide-react';
import { useEffect, useState } from 'react';
import { Area, AreaChart, CartesianGrid, XAxis, YAxis } from 'recharts';
import { PageHeader, PageShell } from './components/page-shell';
import { Alert, AlertDescription, AlertTitle } from './components/ui/alert';
import { Button } from './components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from './components/ui/card';
import {
  ChartContainer,
  ChartLegend,
  ChartLegendContent,
  ChartTooltip,
  ChartTooltipContent,
  type ChartConfig,
} from './components/ui/chart';
import { Progress } from './components/ui/progress';
import { ToggleGroup, ToggleGroupItem } from './components/ui/toggle-group';
import { Tooltip, TooltipContent, TooltipTrigger } from './components/ui/tooltip';
import { appError } from './utils';

type UsageRange = 'today' | '7d' | '30d' | 'all';

type TokenUsage = {
  inputTokens: number;
  cachedInputTokens: number;
  outputTokens: number;
  reasoningOutputTokens: number;
  totalTokens: number;
};

type ModelUsage = {
  model: string;
  tokens: TokenUsage;
  sessionCount: number;
  turnCount: number;
  estimatedCostUsd: number | null;
};

type UsageBucket = {
  key: string;
  label: string;
  totalTokens: number;
};

type UsageAnalytics = {
  total: TokenUsage;
  estimatedCostUsd: number;
  unpricedModelCount: number;
  models: ModelUsage[];
  trend: UsageBucket[];
  skippedFiles: number;
};

const ranges: { value: UsageRange; label: string }[] = [
  { value: 'today', label: '当天' },
  { value: '7d', label: '7 天' },
  { value: '30d', label: '30 天' },
  { value: 'all', label: '全部' },
];

const number = new Intl.NumberFormat('zh-CN');
const compactNumber = new Intl.NumberFormat('en-US', {
  notation: 'compact',
  maximumFractionDigits: 1,
});
const currency = new Intl.NumberFormat('en-US', {
  style: 'currency',
  currency: 'USD',
  minimumFractionDigits: 1,
  maximumFractionDigits: 1,
});

const chartConfig = {
  totalTokens: {
    label: 'Token',
    color: 'var(--chart-1)',
  },
} satisfies ChartConfig;

export default function AnalyticsPage() {
  const [range, setRange] = useState<UsageRange>('today');
  const [refreshKey, setRefreshKey] = useState(0);
  const [analytics, setAnalytics] = useState<UsageAnalytics | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    setAnalytics(null);
    invoke<UsageAnalytics>('get_codex_usage_analytics', { range })
      .then((result) => {
        if (!cancelled) setAnalytics(result);
      })
      .catch((caught) => {
        if (!cancelled) setError(appError(caught));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [range, refreshKey]);

  return (
    <PageShell>
      <PageHeader
        title="统计分析"
        actions={
          <>
            <ToggleGroup
              value={[range]}
              variant="outline"
              size="sm"
              spacing={0}
              aria-label="统计时间范围"
              onValueChange={(values) => {
                const next = values[0] as UsageRange | undefined;
                if (next) setRange(next);
              }}
            >
              {ranges.map((item) => (
                <ToggleGroupItem key={item.value} value={item.value}>
                  {item.label}
                </ToggleGroupItem>
              ))}
            </ToggleGroup>
            <Tooltip>
              <TooltipTrigger render={<span className="inline-flex" />}>
                <Button
                  variant="ghost"
                  size="icon"
                  type="button"
                  onClick={() => setRefreshKey((current) => current + 1)}
                  disabled={loading}
                >
                  <RefreshCw className={loading ? 'animate-spin' : ''} />
                  <span className="sr-only">刷新统计</span>
                </Button>
              </TooltipTrigger>
              <TooltipContent>刷新统计</TooltipContent>
            </Tooltip>
          </>
        }
      />

      <AnalyticsPageContent loading={loading} error={error} analytics={analytics} range={range} />
    </PageShell>
  );
}

function AnalyticsPageContent({
  loading,
  error,
  analytics,
  range,
}: {
  loading: boolean;
  error: string | null;
  analytics: UsageAnalytics | null;
  range: UsageRange;
}) {
  if (loading && !analytics) return <LoadingState />;
  if (error)
    return (
      <div className="px-4 py-7 sm:px-8 lg:px-12">
        <Alert variant="destructive">
          <TriangleAlert />
          <AlertTitle>无法读取统计数据</AlertTitle>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      </div>
    );
  return analytics ? <AnalyticsContent analytics={analytics} range={range} /> : null;
}

function AnalyticsContent({ analytics, range }: { analytics: UsageAnalytics; range: UsageRange }) {
  const cacheHitRate = analytics.total.inputTokens
    ? (analytics.total.cachedInputTokens / analytics.total.inputTokens) * 100
    : 0;

  return (
    <div className="flex flex-col gap-8 px-4 pt-6 pb-10 sm:px-8 lg:px-12">
      {analytics.skippedFiles > 0 && (
        <Alert>
          <TriangleAlert />
          <AlertTitle>部分记录未统计</AlertTitle>
          <AlertDescription>有 {analytics.skippedFiles} 个会话文件无法读取。</AlertDescription>
        </Alert>
      )}

      <section aria-label="用量概览" className="grid gap-3 sm:grid-cols-2 md:grid-cols-4">
        <MetricCard label="总 Token" value={analytics.total.totalTokens} detail="当前范围内用量" />
        <CostMetricCard
          value={analytics.estimatedCostUsd}
          unpricedModelCount={analytics.unpricedModelCount}
        />
        <MetricCard
          label="输入 Token"
          value={analytics.total.inputTokens}
          detail={`缓存命中率 ${cacheHitRate.toFixed(1)}%`}
        />
        <MetricCard
          label="输出 Token"
          value={analytics.total.outputTokens}
          detail={`推理输出 ${number.format(analytics.total.reasoningOutputTokens)}`}
        />
      </section>

      <section>
        <SectionHeader title="Token 趋势" description={trendDescription(range)} />
        {analytics.total.totalTokens ? <UsageChart buckets={analytics.trend} /> : <EmptyState />}
      </section>

      <section>
        <SectionHeader title="模型用量" description="按总 Token 从高到低排列" />
        {analytics.models.length ? (
          <div className="border-y border-border">
            {analytics.models.map((model) => (
              <ModelRow key={model.model} usage={model} totalTokens={analytics.total.totalTokens} />
            ))}
          </div>
        ) : (
          <EmptyState />
        )}
      </section>
    </div>
  );
}

function CostMetricCard({
  value,
  unpricedModelCount,
}: {
  value: number;
  unpricedModelCount: number;
}) {
  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle>预估费用</CardTitle>
      </CardHeader>
      <CardContent>
        <strong className="text-2xl font-semibold tabular-nums" title={currency.format(value)}>
          {currency.format(value)}
        </strong>
        <p className="mt-2 text-xs text-muted-foreground">
          {unpricedModelCount ? `${unpricedModelCount} 个模型未计入` : '全部模型已定价'}
        </p>
      </CardContent>
    </Card>
  );
}

function MetricCard({ label, value, detail }: { label: string; value: number; detail: string }) {
  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle>{label}</CardTitle>
      </CardHeader>
      <CardContent>
        <strong className="text-2xl font-semibold tabular-nums" title={number.format(value)}>
          {compactNumber.format(value)}
        </strong>
        <p className="mt-2 text-xs text-muted-foreground">{detail}</p>
      </CardContent>
    </Card>
  );
}

function UsageChart({ buckets }: { buckets: UsageBucket[] }) {
  return (
    <ChartContainer config={chartConfig} className="mt-4 aspect-auto h-[250px] w-full">
      <AreaChart accessibilityLayer data={buckets}>
        <defs>
          <linearGradient id="fillTokens" x1="0" y1="0" x2="0" y2="1">
            <stop offset="5%" stopColor="var(--color-totalTokens)" stopOpacity={0.8} />
            <stop offset="95%" stopColor="var(--color-totalTokens)" stopOpacity={0.1} />
          </linearGradient>
        </defs>
        <CartesianGrid vertical={false} />
        <XAxis dataKey="label" tickLine={false} axisLine={false} tickMargin={8} minTickGap={32} />
        <YAxis
          dataKey="totalTokens"
          domain={[0, 'auto']}
          allowDecimals={false}
          tickFormatter={(value: number) => compactNumber.format(value)}
          tickLine={false}
          axisLine={false}
          tickMargin={8}
          width="auto"
        />
        <ChartTooltip
          cursor={false}
          content={
            <ChartTooltipContent indicator="dot" labelFormatter={(value) => String(value)} />
          }
        />
        <Area
          dataKey="totalTokens"
          type="monotoneX"
          fill="url(#fillTokens)"
          stroke="var(--color-totalTokens)"
        />
        <ChartLegend content={<ChartLegendContent />} />
      </AreaChart>
    </ChartContainer>
  );
}

function ModelRow({ usage, totalTokens }: { usage: ModelUsage; totalTokens: number }) {
  const share = totalTokens ? (usage.tokens.totalTokens / totalTokens) * 100 : 0;
  const metrics = [
    ['输入', usage.tokens.inputTokens],
    ['缓存输入', usage.tokens.cachedInputTokens],
    ['输出', usage.tokens.outputTokens],
    ['推理输出', usage.tokens.reasoningOutputTokens],
  ] as const;

  return (
    <article className="flex flex-col gap-3 border-b border-border py-5 last:border-b-0">
      <div className="flex items-baseline justify-between gap-4">
        <strong className="truncate text-sm font-medium" title={usage.model}>
          {usage.model}
        </strong>
        <div className="flex shrink-0 items-baseline gap-4">
          <span className="text-sm font-medium tabular-nums text-muted-foreground">
            {usage.estimatedCostUsd === null ? '未定价' : currency.format(usage.estimatedCostUsd)}
          </span>
          <span className="text-sm font-semibold tabular-nums">
            {number.format(usage.tokens.totalTokens)}
            <span className="ml-1 font-normal text-muted-foreground">Token</span>
          </span>
        </div>
      </div>
      <Progress value={share} aria-label={`${usage.model} 占比 ${share.toFixed(1)}%`} />
      <div className="grid gap-3 text-xs sm:grid-cols-3 xl:grid-cols-6">
        {metrics.map(([label, value]) => (
          <TokenDetail key={label} label={String(label)} value={Number(value)} />
        ))}
        <TokenDetail label="会话" value={usage.sessionCount} />
        <TokenDetail label="轮次" value={usage.turnCount} />
      </div>
    </article>
  );
}

function TokenDetail({ label, value }: { label: string; value: number }) {
  return (
    <div className="min-w-0">
      <span className="block text-muted-foreground">{label}</span>
      <span className="mt-1 block truncate font-medium tabular-nums" title={number.format(value)}>
        {number.format(value)}
      </span>
    </div>
  );
}

function SectionHeader({ title, description }: { title: string; description: string }) {
  return (
    <div>
      <h2 className="text-base font-semibold">{title}</h2>
      <p className="mt-1 text-xs text-muted-foreground">{description}</p>
    </div>
  );
}

function LoadingState() {
  return (
    <div className="grid min-h-72 place-items-center text-muted-foreground">
      <LoaderCircle className="animate-spin" />
    </div>
  );
}

function EmptyState() {
  return (
    <div className="mt-4 border-y border-border py-12 text-center text-sm text-muted-foreground">
      当前范围内暂无 Token 记录
    </div>
  );
}

function trendDescription(range: UsageRange) {
  if (range === 'today') return '按小时汇总';
  if (range === 'all') return '按月汇总';
  return '按天汇总';
}
