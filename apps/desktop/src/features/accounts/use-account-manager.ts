import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { FormEvent } from 'react';
import { useCallback, useEffect, useState } from 'react';
import { toast } from 'sonner';
import { appError } from '../../utils';
import type { AddMode, AppStatus, OAuthProgress, PendingConfirm, Profile } from './types';

export function useAccountManager() {
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

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<AppStatus>('get_app_status'));
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => void refresh(), [refresh]);

  useEffect(() => {
    const unlisten = listen<OAuthProgress>('oauth-progress', ({ payload }) => {
      setOauthMessage(payload.message);
      if (payload.stage === 'success') {
        setBusy(null);
        closeAddDialog();
        toast.success(payload.message);
        void refresh();
      } else if (payload.stage === 'error') {
        setBusy(null);
        toast.error(payload.message);
      }
    });
    return () => void unlisten.then((cleanup) => cleanup());
  }, [refresh]);

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
      setBusy(null);
      closeAddDialog();
    }
  }

  async function refreshAllAccounts() {
    const profiles = (status?.profiles ?? []).filter((profile) => profile.accountType === 'oauth');
    if (!profiles.length) {
      toast.info('中转站账户不支持额度查询。');
      return;
    }
    setBusy('refresh:all');
    const results = await Promise.allSettled(
      profiles.map((profile) =>
        invoke<Profile>('refresh_profile_usage', { profileId: profile.id }),
      ),
    );
    await refresh();
    const failed = results.filter((result) => result.status === 'rejected');
    if (failed.length) toast.error(`${failed.length} 个账户信息刷新失败。`);
    else toast.success('账户信息已刷新。');
    setBusy(null);
  }

  async function refreshAccount(profile: Profile) {
    if (profile.accountType === 'relay') return;
    setBusy(`refresh:${profile.id}`);
    try {
      await invoke<Profile>('refresh_profile_usage', { profileId: profile.id });
      toast.success(`${profile.alias} 的账户信息已刷新。`);
      await refresh();
    } catch (error) {
      toast.error(appError(error));
    } finally {
      setBusy(null);
    }
  }

  async function switchTo(profile: Profile, force = false) {
    setBusy(`switch:${profile.id}`);
    try {
      await invoke<Profile>('switch_profile', { profileId: profile.id, force });
      toast.success(`已切换到 ${profile.alias}。`);
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
      const profile = await invoke<Profile>('import_current_profile', { alias: null });
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
    if (addMode === 'browser') {
      setBusy('oauth');
      setOauthMessage('正在准备浏览器授权。');
      try {
        await invoke('start_oauth_add', { alias: alias || null, activate: false });
      } catch (error) {
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
            });
      closeAddDialog();
      toast.success(`已添加 ${profile.alias}。`);
      await refresh();
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
            }
          : { profileId: editing.id, alias: editingAlias, authJson: editingAuthJson },
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
      await invoke('delete_profile', { profileId: profile.id });
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
      await invoke('reorder_profiles', { profileIds: profiles.map((profile) => profile.id) });
    } catch (error) {
      toast.error(appError(error));
      await refresh();
    }
  }

  return {
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
    refresh,
    closeAddDialog,
    closeEditor,
    cancelOAuth,
    refreshAllAccounts,
    refreshAccount,
    switchTo,
    importCurrent,
    submitAdd,
    openEditor,
    saveProfile,
    deleteProfile,
    reorderProfiles,
  };
}
