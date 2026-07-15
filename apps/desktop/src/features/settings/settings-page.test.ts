import { describe, expect, it } from 'vitest';
import { isCliUpgradeAvailable } from './cli-environment';

describe('isCliUpgradeAvailable', () => {
  it('only reports an upgrade when installed and latest versions differ', () => {
    expect(
      isCliUpgradeAvailable({
        installed: true,
        installedVersion: '1.0.0',
        latestVersion: '1.1.0',
        installMethod: 'standalone',
      }),
    ).toBe(true);
    expect(
      isCliUpgradeAvailable({
        installed: true,
        installedVersion: '1.1.0',
        latestVersion: '1.1.0',
        installMethod: 'standalone',
      }),
    ).toBe(false);
  });
});
