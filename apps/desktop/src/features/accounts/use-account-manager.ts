import type { FormEvent } from 'react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { invoke, listenOAuthProgress } from '../../backend';
import { appError } from '../../utils';
import type {
  AddMode,
  AccountProduct,
  AppStatus,
  OAuthProgress,
  PendingConfirm,
  Profile,
  ResetCredits,
  UsageRefreshResult,
} from './types';

export function useAccountManager(product: AccountProduct) {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [addMode, setAddMode] = useState<AddMode>('browser');
  const [alias, setAlias] = useState('');
  const [authJson, setAuthJson] = useState('');
  const [relayApiKey, setRelayApiKey] = useState('');
  const [relayApiBaseUrl, setRelayApiBaseUrl] = useState('');
  const [showRelayApiKey, setShowRelayApiKey] = useState(false);
  const [oauthMessage, setOauthMessage] = useState<string | null>(null);
  const [editing, setEditing] = useState<Profile | null>(null);
  const [editingAlias, setEditingAlias] = useState('');
  const [editingAuthJson, setEditingAuthJson] = useState('');
  const [editingRelayApiKey, setEditingRelayApiKey] = useState('');
  const [editingRelayApiBaseUrl, setEditingRelayApiBaseUrl] = useState('');
  const [showEditingRelayApiKey, setShowEditingRelayApiKey] = useState(false);
  const [confirm, setConfirm] = useState<PendingConfirm>(null);
  const [resetCreditsProfile, setResetCreditsProfile] = useState<Profile | null>(null);
  const [resetCredits, setResetCredits] = useState<ResetCredits | null>(null);
  const [quotaProfileId, setQuotaProfileId] = useState<string | null>(null);
  const refreshRequestRef = useRef(0);
  const oauthCleanupRef = useRef<(() => void) | null>(null);

  const refresh = useCallback(
    async (showError = true) => {
      const request = ++refreshRequestRef.current;
      try {
        const next = await invoke<AppStatus>('get_app_status', { product });
        if (request === refreshRequestRef.current) setStatus(next);
      } catch (error) {
        if (showError && request === refreshRequestRef.current) toast.error(appError(error));
      } finally {
        if (request === refreshRequestRef.current) setLoading(false);
      }
    },
    [product],
  );

  const refreshNewAccount = useCallback(
    async (profile: Profile) => {
      try {
        if (profile.accountType === 'oauth' && product !== 'claude') {
          await invoke<UsageRefreshResult>('refresh_profile_usage', { profileId: profile.id });
        }
      } catch (error) {
        toast.error(`账号已添加，但信息刷新失败：${appError(error)}`);
      }
      await refresh();
    },
    [product, refresh],
  );

  useEffect(() => {
    setLoading(true);
    setStatus(null);
    setAddOpen(false);
    setAddMode('browser');
    setQuotaProfileId(null);
    void refresh();
    const statusTimer =
      product === 'codex' ? window.setInterval(() => void refresh(false), 10_000) : undefined;
    if (product === 'codex') {
      void invoke('refresh_due_profile_usage', { immediate: true })
        .then(() => refresh(false))
        .catch(() => {});
    }
    return () => {
      if (statusTimer !== undefined) window.clearInterval(statusTimer);
      refreshRequestRef.current += 1;
    };
  }, [product, refresh]);

  const stopOAuthProgress = useCallback(() => {
    oauthCleanupRef.current?.();
    oauthCleanupRef.current = null;
  }, []);

  useEffect(() => stopOAuthProgress, [stopOAuthProgress]);

  async function startOAuthProgress() {
    stopOAuthProgress();
    oauthCleanupRef.current = await listenOAuthProgress<OAuthProgress>((payload) => {
      setOauthMessage(payload.message);
      if (payload.stage === 'success') {
        stopOAuthProgress();
        setBusy(null);
        closeAddDialog();
        toast.success(payload.message);
        void (payload.profile ? refreshNewAccount(payload.profile) : refresh());
      } else if (payload.stage === 'error') {
        stopOAuthProgress();
        setBusy(null);
        toast.error(payload.message);
      } else if (payload.stage === 'cancelled') {
        stopOAuthProgress();
        setBusy(null);
        closeAddDialog();
      }
    });
  }

  const activeProfile =
    status?.detectedProfile ?? status?.profiles.find((profile) => profile.isActive) ?? null;

  function closeAddDialog() {
    setAddOpen(false);
    setAlias('');
    setAuthJson('');
    setRelayApiKey('');
    setRelayApiBaseUrl('');
    setShowRelayApiKey(false);
    setOauthMessage(null);
  }

  function closeEditor() {
    setEditing(null);
    setEditingAlias('');
    setEditingAuthJson('');
    setEditingRelayApiKey('');
    setEditingRelayApiBaseUrl('');
    setShowEditingRelayApiKey(false);
  }

  async function cancelOAuth() {
    try {
      await invoke('cancel_oauth_add');
    } catch (error) {
      toast.error(appError(error));
    } finally {
      stopOAuthProgress();
      setBusy(null);
      closeAddDialog();
    }
  }

  async function refreshAllAccounts() {
    const profiles = (status?.profiles ?? []).filter(
      (profile) => profile.accountType === 'oauth' && (product !== 'claude' || profile.isRenewable),
    );
    if (!profiles.length) {
      toast.info(product === 'claude' ? '没有可更新的登录令牌。' : '没有可刷新的账户。');
      return;
    }
    setBusy('refresh:all');
    const results = await Promise.allSettled(
      profiles.map((profile) =>
        invoke<UsageRefreshResult>('refresh_profile_usage', { profileId: profile.id }),
      ),
    );
    await refresh();
    const failed = results.filter((result) => result.status === 'rejected');
    if (failed.length)
      toast.error(`${failed.length} 个账户${product === 'claude' ? '令牌更新' : '信息刷新'}失败。`);
    else if (results.some((result) => result.status === 'fulfilled' && result.value.refreshed))
      toast.success(product === 'claude' ? '登录令牌已更新。' : '账户信息已刷新。');
    else toast.info('账户信息刚刚已刷新。');
    setBusy(null);
  }

  async function refreshAccount(profile: Profile) {
    if (profile.accountType === 'relay') return;
    if (product === 'claude' && !profile.isRenewable) {
      toast.info('此 Token 无法续期，请重新进行浏览器授权。');
      return;
    }
    setBusy(`refresh:${profile.id}`);
    try {
      const result = await invoke<UsageRefreshResult>('refresh_profile_usage', {
        profileId: profile.id,
      });
      if (result.refreshed) {
        toast.success(
          product === 'claude'
            ? `${profile.alias} 的登录令牌已更新。`
            : `${profile.alias} 的账户信息已刷新。`,
        );
      } else {
        toast.info(`${profile.alias} 的账户信息刚刚已刷新。`);
      }
      await refresh();
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setBusy(null);
    }
  }

  async function viewResetCredits(profile: Profile) {
    setResetCreditsProfile(profile);
    setResetCredits(null);
    setBusy(`reset-credits:${profile.id}`);
    try {
      const result = await invoke<ResetCredits>('get_profile_reset_credits', {
        profileId: profile.id,
      });
      setResetCredits(result);
      setStatus((current) =>
        current
          ? {
              ...current,
              profiles: current.profiles.map((item) =>
                item.id === profile.id
                  ? { ...item, resetCreditsAvailableCount: result.availableCount }
                  : item,
              ),
            }
          : current,
      );
    } catch (error) {
      toast.error(appError(error));
      setResetCreditsProfile(null);
    } finally {
      setBusy(null);
    }
  }

  async function switchTo(profile: Profile, force = false) {
    setBusy(`switch:${profile.id}`);
    try {
      await invoke<Profile>('switch_profile', { product, profileId: profile.id, force });
      toast.success(
        product === 'antigravity'
          ? `已切换到 ${profile.alias}，新启动的 agy 会话生效。`
          : product === 'claude'
            ? `已切换到 ${profile.alias}，新启动的 claude 会话生效。`
            : `已切换到 ${profile.alias}。`,
      );
      setConfirm(null);
      await refresh();
    } catch (error) {
      const message = appError(error);
      if (!force && message.includes('工具外')) setConfirm({ kind: 'force-switch', profile });
      else toast.error(message);
    } finally {
      setBusy(null);
    }
  }

  async function importCurrent() {
    setBusy('import');
    try {
      const profile = await invoke<Profile>('import_current_profile', { product, alias: null });
      toast.success(`已同步 ${profile.alias}。`);
      await refresh();
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setBusy(null);
    }
  }

  async function submitAdd(event: FormEvent) {
    event.preventDefault();
    if (addMode === 'browser' || (product !== 'codex' && product !== 'claude')) {
      setBusy('oauth');
      setOauthMessage('正在准备浏览器授权。');
      try {
        await startOAuthProgress();
        await invoke('start_oauth_add', { product, alias: alias || null, activate: false });
      } catch (error) {
        stopOAuthProgress();
        setBusy(null);
        toast.error(appError(error));
      }
      return;
    }

    setBusy(addMode === 'paste' ? 'auth-json' : 'relay');
    try {
      const profile =
        addMode === 'paste'
          ? await invoke<Profile>('import_auth_json', {
              authJson,
              alias: alias || null,
              activate: false,
            })
          : await invoke<Profile>('add_relay_profile', {
              apiKey: relayApiKey,
              apiBaseUrl: relayApiBaseUrl,
              alias: alias || null,
              activate: false,
              product,
            });
      closeAddDialog();
      toast.success(`已添加 ${profile.alias}。`);
      await refreshNewAccount(profile);
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setBusy(null);
    }
  }

  async function openEditor(profile: Profile) {
    if (profile.accountType === 'relay') {
      setEditing(profile);
      setEditingAlias(profile.alias);
      setEditingRelayApiKey('');
      setEditingRelayApiBaseUrl(profile.apiBaseUrl ?? '');
      return;
    }
    if (product !== 'codex') {
      setEditing(profile);
      setEditingAlias(profile.alias);
      return;
    }
    setBusy(`edit:${profile.id}`);
    try {
      setEditingAuthJson(await invoke<string>('get_profile_auth', { profileId: profile.id }));
      setEditing(profile);
      setEditingAlias(profile.alias);
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setBusy(null);
    }
  }

  async function saveProfile(event: FormEvent) {
    event.preventDefault();
    if (!editing) return;
    setBusy(`edit:${editing.id}`);
    try {
      await invoke(
        editing.accountType === 'relay' ? 'update_relay_profile' : 'update_profile',
        editing.accountType === 'relay'
          ? {
              profileId: editing.id,
              alias: editingAlias,
              apiKey: editingRelayApiKey.trim() || null,
              apiBaseUrl: editingRelayApiBaseUrl,
              product,
            }
          : {
              product,
              profileId: editing.id,
              alias: editingAlias,
              ...(product === 'codex' ? { authJson: editingAuthJson } : {}),
            },
      );
      closeEditor();
      toast.success('账户信息已保存。');
      await refresh();
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setBusy(null);
    }
  }

  async function deleteProfile(profile: Profile) {
    setBusy(`delete:${profile.id}`);
    try {
      await invoke('delete_profile', { product, profileId: profile.id });
      setConfirm(null);
      toast.success(`已移除 ${profile.alias}。`);
      await refresh();
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setBusy(null);
    }
  }

  async function reorderProfiles(profiles: Profile[]) {
    if (!status || profiles === status.profiles) return;
    setStatus({ ...status, profiles });
    try {
      await invoke('reorder_profiles', {
        product,
        profileIds: profiles.map((profile) => profile.id),
      });
    } catch (error) {
      toast.error(appError(error));
      await refresh();
    }
  }

  return {
    product,
    status,
    loading,
    busy,
    addOpen,
    addMode,
    alias,
    authJson,
    relayApiKey,
    relayApiBaseUrl,
    showRelayApiKey,
    oauthMessage,
    editing,
    editingAlias,
    editingAuthJson,
    editingRelayApiKey,
    editingRelayApiBaseUrl,
    showEditingRelayApiKey,
    confirm,
    resetCreditsProfile,
    resetCredits,
    quotaProfile: status?.profiles.find((profile) => profile.id === quotaProfileId) ?? null,
    activeProfile,
    setAddOpen,
    setAddMode,
    setAlias,
    setAuthJson,
    setRelayApiKey,
    setRelayApiBaseUrl,
    setShowRelayApiKey,
    setEditingAlias,
    setEditingAuthJson,
    setEditingRelayApiKey,
    setEditingRelayApiBaseUrl,
    setShowEditingRelayApiKey,
    setConfirm,
    setResetCreditsProfile,
    setQuotaProfileId,
    refresh,
    closeAddDialog,
    closeEditor,
    cancelOAuth,
    refreshAllAccounts,
    refreshAccount,
    viewResetCredits,
    switchTo,
    importCurrent,
    submitAdd,
    openEditor,
    saveProfile,
    deleteProfile,
    reorderProfiles,
  };
}
