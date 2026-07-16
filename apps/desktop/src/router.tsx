import {
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
  lazyRouteComponent,
  redirect,
} from '@tanstack/react-router';
import AccountsPage from './features/accounts/accounts-page';

const AppShell = lazyRouteComponent(() => import('./components/app-shell'), 'AppShell');
const MainLayout = lazyRouteComponent(() => import('./components/app-shell'), 'MainLayout');
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
const SettingsLayout = lazyRouteComponent(() => import('./features/settings/settings-layout'));
const SettingsPage = lazyRouteComponent(() => import('./features/settings/settings-page'));
const AboutPage = lazyRouteComponent(
  () => import('./features/settings/settings-page'),
  'AboutPage',
);

const rootRoute = createRootRoute({ component: AppShell });

const mainLayoutRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: 'main',
  component: MainLayout,
});

const indexRoute = createRoute({
  getParentRoute: () => mainLayoutRoute,
  path: '/',
  beforeLoad: () => {
    throw redirect({ to: '/accounts' });
  },
});

const accountsRoute = createRoute({
  getParentRoute: () => mainLayoutRoute,
  path: '/accounts',
  component: AccountsPage,
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings',
  component: SettingsLayout,
  beforeLoad: ({ location }) => {
    if (location.pathname === '/settings') throw redirect({ to: '/settings/general' });
  },
});

const generalSettingsRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: '/general',
  component: SettingsPage,
});

const billingRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: '/billing',
  component: BillingPage,
});

const aboutRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: '/about',
  component: AboutPage,
});

const configRoute = createRoute({
  getParentRoute: () => mainLayoutRoute,
  path: '/config',
  component: ConfigPage,
});

const promptsRoute = createRoute({
  getParentRoute: () => mainLayoutRoute,
  path: '/prompts',
  component: PromptsPage,
});

const newPromptRoute = createRoute({
  getParentRoute: () => mainLayoutRoute,
  path: '/prompts/new',
  component: NewPromptPage,
});

const editPromptRoute = createRoute({
  getParentRoute: () => mainLayoutRoute,
  path: '/prompts/edit/$profileId',
  component: EditPromptPage,
});

const sessionsRoute = createRoute({
  getParentRoute: () => mainLayoutRoute,
  path: '/sessions',
  component: SessionsPage,
});

const analyticsRoute = createRoute({
  getParentRoute: () => mainLayoutRoute,
  path: '/analytics',
  component: AnalyticsPage,
});

export const router = createRouter({
  routeTree: rootRoute.addChildren([
    mainLayoutRoute.addChildren([
      indexRoute,
      accountsRoute,
      sessionsRoute,
      analyticsRoute,
      promptsRoute,
      newPromptRoute,
      editPromptRoute,
      configRoute,
    ]),
    settingsRoute.addChildren([generalSettingsRoute, billingRoute, aboutRoute]),
  ]),
  history: createHashHistory(),
});

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router;
  }
}
