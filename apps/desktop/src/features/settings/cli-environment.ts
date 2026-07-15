export type CliEnvironment = {
  installed: boolean;
  installedVersion: string | null;
  latestVersion: string | null;
  installMethod: 'brew' | 'npm' | 'pnpm' | 'bun' | 'standalone' | 'unknown';
};

export function isCliUpgradeAvailable(environment?: CliEnvironment) {
  return Boolean(
    environment?.installed &&
    environment.installedVersion &&
    environment.latestVersion &&
    environment.installedVersion !== environment.latestVersion,
  );
}
