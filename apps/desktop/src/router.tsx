import {
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
  lazyRouteComponent,
} from '@tanstack/react-router';
import { AppLayout } from './components/page-shell';
import AccountsPage from './features/accounts/accounts-page';

const AnalyticsPage = lazyRouteComponent(() => import('./features/analytics/analytics-page'));
const BillingPage = lazyRouteComponent(() => import('./features/billing/billing-page'));
const ConfigPage = lazyRouteComponent(() => import('./features/config/config-page'));
const PromptsPage = lazyRouteComponent(() => import('./features/prompts/prompts-page'));
const NewPromptPage = lazyRouteComponent(
  () => import('./features/prompts/prompt-editor-page'),
  'NewPromptPage',
);
const EditPromptPage = lazyRouteComponent(
  () => import('./features/prompts/prompt-editor-page'),
  'EditPromptPage',
);
const SessionsPage = lazyRouteComponent(() => import('./features/sessions/sessions-page'));
const SettingsPage = lazyRouteComponent(() => import('./features/settings/settings-page'));
const AboutPage = lazyRouteComponent(
  () => import('./features/settings/settings-page'),
  'AboutPage',
);

const rootRoute = createRootRoute({ component: AppLayout });

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  component: AccountsPage,
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings',
  component: SettingsPage,
});

const billingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings/billing',
  component: BillingPage,
});

const aboutRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings/about',
  component: AboutPage,
});

const configRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/config',
  component: ConfigPage,
});

const promptsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/prompts',
  component: PromptsPage,
});

const newPromptRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/prompts/new',
  component: NewPromptPage,
});

const editPromptRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/prompts/edit/$profileId',
  component: EditPromptPage,
});

const sessionsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/sessions',
  component: SessionsPage,
});

const analyticsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/analytics',
  component: AnalyticsPage,
});

export const router = createRouter({
  routeTree: rootRoute.addChildren([
    indexRoute,
    sessionsRoute,
    analyticsRoute,
    promptsRoute,
    newPromptRoute,
    editPromptRoute,
    settingsRoute,
    billingRoute,
    aboutRoute,
    configRoute,
  ]),
  history: createHashHistory(),
});

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router;
  }
}
