# Cortana

本地 Tauri 桌面工具，用于管理 Codex、Claude、Antigravity 和 Grok CLI 账号。支持保存、排序和切换多个本地认证档案。

## 开发环境

通用要求：

- Node.js 22
- pnpm 11.8.0
- Rust 1.95.0
- 支持 `app-server` 的 Codex CLI（会话管理功能需要）
- Grok CLI（Grok 账号管理需要）

系统依赖：

- Windows：Visual Studio Build Tools 2022，安装“使用 C++ 的桌面开发”和 Windows SDK；Rust 使用 `x86_64-pc-windows-msvc`；安装 WebView2 Runtime。
- macOS：运行 `xcode-select --install` 安装 Xcode Command Line Tools。
- Debian/Ubuntu：

```sh
sudo apt update
sudo apt install build-essential curl wget file libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev patchelf
```

安装依赖并启动：

```sh
corepack enable
pnpm install --frozen-lockfile
pnpm dev
```

仅启动 Web 前端：

```sh
pnpm --filter @cortana/desktop dev:web
```

Web 访问默认关闭，可在桌面应用的“设置 → 常规 → Web 访问”中启用，默认地址为 `http://127.0.0.1:11456`。启用后可直接访问，无需 Token 或其他权限校验。

Codex 默认使用 `~/.codex/auth.json`，Claude 使用 `~/.claude/settings.json`，Grok 默认使用 `$GROK_HOME/auth.json` 或 `~/.grok/auth.json`。Claude 中转站账户使用 `ANTHROPIC_BASE_URL` 和 `ANTHROPIC_AUTH_TOKEN`；所有档案仅保存在本机用户级 SQLite 数据库，包含可用认证令牌或 API Key；请勿把该数据库上传到同步盘或代码仓库。

相关文档：

- [Codex auth.json 结构](docs/auth-json.md)
- [OpenAI 接口清单](docs/openai-interfaces.md)
- [Grok 官方接口清单](docs/grok-interfaces.md)
- [常见开发问题](docs/common-development-issues.md)

## 构建

构建必须在目标操作系统上执行。产物位于 `src-tauri/target/release/bundle/`。

当前系统的安装包：

```sh
pnpm build
```

Windows x64 MSI：

```sh
pnpm tauri build --target x86_64-pc-windows-msvc --bundles msi
```

macOS universal `.pkg`：

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
sh scripts/package-macos.sh
```

正式发布前还需配置对应平台的代码签名；当前 CI 发布目标为 Windows x64 和 macOS arm64。

## FAQ / 常见问题

### macOS 提示“无法验证 Cortana.app 恶意软件”

如果在 macOS 上打开应用时提示“Apple无法验证 Cortana.app 是否包含可能危害Mac安全或泄漏隐私的恶意软件”，可在终端运行以下命令清除隔离标记：

```sh
sudo xattr -r -d com.apple.quarantine /Applications/Cortana.app
```

> **注**：如果 `.app` 文件存放在其他路径（如未移动至 `/Applications`），请将命令中的路径替换为实际的 `Cortana.app` 路径。
