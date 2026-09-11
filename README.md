# Cortana

本地 Tauri 桌面工具，用于管理 Codex、Claude、Antigravity 和 Grok CLI 账号。支持保存、排序和切换多个本地认证档案。


## FAQ / 常见问题

### macOS 提示“无法验证 Cortana.app 恶意软件”

如果在 macOS 上打开应用时提示“Apple无法验证 Cortana.app 是否包含可能危害Mac安全或泄漏隐私的恶意软件”，可在终端运行以下命令清除隔离标记：

```sh
sudo xattr -r -d com.apple.quarantine /Applications/Cortana.app
```

> **注**：如果 `.app` 文件存放在其他路径（如未移动至 `/Applications`），请将命令中的路径替换为实际的 `Cortana.app` 路径。
