#!/usr/bin/env bash

set -euo pipefail

cd "$(dirname "$0")/.."

VERSION=${1:-}

if [[ "$(git branch --show-current)" != "main" ]]; then
  echo "发布只能在 main 分支执行。" >&2
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "工作区存在未提交改动，请先提交或清理。" >&2
  exit 1
fi

git fetch origin main

if [[ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]]; then
  echo "本地 main 与 origin/main 不一致，请先同步。" >&2
  exit 1
fi

if ! command -v gh >/dev/null 2>&1 || ! gh auth status >/dev/null 2>&1; then
  echo "请先安装 GitHub CLI 并执行 gh auth login。" >&2
  exit 1
fi

CURRENT_VERSION=$(node -e "console.log(require('./package.json').version)")
if [[ ! "$CURRENT_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "当前版本号格式错误：$CURRENT_VERSION" >&2
  exit 1
fi

if [[ -z "$VERSION" ]]; then
  if [[ ! -t 0 ]]; then
    echo "非交互环境请使用 pnpm release <版本号>。" >&2
    exit 1
  fi

  IFS=. read -r MAJOR MINOR PATCH <<<"$CURRENT_VERSION"
  PATCH_VERSION="$MAJOR.$MINOR.$((PATCH + 1))"
  MINOR_VERSION="$MAJOR.$((MINOR + 1)).0"
  MAJOR_VERSION="$((MAJOR + 1)).0.0"

  echo "当前版本：$CURRENT_VERSION"
  select OPTION in \
    "补丁版本 Patch：$PATCH_VERSION" \
    "次版本 Minor：$MINOR_VERSION" \
    "主版本 Major：$MAJOR_VERSION" \
    "取消发布"; do
    case "$REPLY" in
      1) VERSION=$PATCH_VERSION ;;
      2) VERSION=$MINOR_VERSION ;;
      3) VERSION=$MAJOR_VERSION ;;
      4)
        echo "已取消发布。"
        exit 0
        ;;
      *)
        echo "请输入 1-4。" >&2
        continue
        ;;
    esac
    break
  done

  read -r -p "确认发布 v${VERSION}？[y/N] " CONFIRM
  if [[ ! "$CONFIRM" =~ ^[Yy]$ ]]; then
    echo "已取消发布。"
    exit 0
  fi
fi

if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "版本号格式错误，应为 X.Y.Z。" >&2
  exit 1
fi

TAG="v$VERSION"

if git show-ref --verify --quiet "refs/tags/$TAG" ||
  git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
  echo "Tag $TAG 已存在。" >&2
  exit 1
fi

node - "$VERSION" <<'NODE'
const fs = require('node:fs');

const version = process.argv[2];
const jsonFiles = ['package.json', 'apps/desktop/package.json', 'src-tauri/tauri.conf.json'];
const jsonDocuments = jsonFiles.map((file) => [file, JSON.parse(fs.readFileSync(file, 'utf8'))]);
const cargoFile = 'src-tauri/Cargo.toml';
const cargo = fs.readFileSync(cargoFile, 'utf8');
const cargoMatch = cargo.match(/^version = "([^"]+)"$/m);

if (!cargoMatch) throw new Error('无法读取 Cargo.toml 版本号。');

const currentVersions = [...jsonDocuments.map(([, document]) => document.version), cargoMatch[1]];
if (new Set(currentVersions).size !== 1) {
  throw new Error(`当前版本号不一致：${currentVersions.join(', ')}`);
}

const current = currentVersions[0];
const compare = (left, right) => {
  const a = left.split('.').map(Number);
  const b = right.split('.').map(Number);
  for (let index = 0; index < 3; index += 1) {
    if (a[index] !== b[index]) return a[index] - b[index];
  }
  return 0;
};

if (compare(version, current) <= 0) {
  throw new Error(`新版本 ${version} 必须高于当前版本 ${current}。`);
}

for (const [file, document] of jsonDocuments) {
  document.version = version;
  fs.writeFileSync(file, `${JSON.stringify(document, null, 2)}\n`);
}
fs.writeFileSync(cargoFile, cargo.replace(cargoMatch[0], `version = "${version}"`));
NODE

pnpm --filter @cortana/desktop build:web
cargo test --manifest-path src-tauri/Cargo.toml

git add \
  package.json \
  apps/desktop/package.json \
  src-tauri/Cargo.toml \
  src-tauri/Cargo.lock \
  src-tauri/tauri.conf.json
git commit -m "chore(release): $TAG"
git tag -a "$TAG" -m "Cortana $TAG"
git push --atomic origin main "$TAG"

gh release create "$TAG" \
  --verify-tag \
  --title "Cortana $TAG" \
  --generate-notes

echo "已发布 ${TAG}，GitHub Actions 将自动构建并上传正式安装包。"
