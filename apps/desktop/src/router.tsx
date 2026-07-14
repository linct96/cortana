import {
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
  lazyRouteComponent,
} from '@tanstack/react-router';
import App from './App';
import { AppLayout } from './components/page-shell';

const AnalyticsPage = lazyRouteComponent(() => import('./AnalyticsPage'));
const BillingPage = lazyRouteComponent(() => import('./BillingPage'));
const ConfigPage = lazyRouteComponent(() => import('./ConfigPage'));
const PromptsPage = lazyRouteComponent(() => import('./PromptsPage'));
const NewPromptPage = lazyRouteComponent(() => import('./PromptEditorPage'), 'NewPromptPage');
const EditPromptPage = lazyRouteComponent(() => import('./PromptEditorPage'), 'EditPromptPage');
const SessionsPage = lazyRouteComponent(() => import('./SessionsPage'));
const SettingsPage = lazyRouteComponent(() => import('./SettingsPage'));
const AboutPage = lazyRouteComponent(() => import('./SettingsPage'), 'AboutPage');

const rootRoute = createRootRoute({ component: AppLayout });

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  component: App,
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
