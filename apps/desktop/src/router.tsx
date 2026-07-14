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
