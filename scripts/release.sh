#!/usr/bin/env bash

set -euo pipefail

cd "$(dirname "$0")/.."

VERSION=${1:-}
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "用法：pnpm release <版本号，例如 1.0.0>" >&2
  exit 1
fi

TAG="v$VERSION"

if [[ "$(git branch --show-current)" != "main" ]]; then
  echo "发布只能在 main 分支执行。" >&2
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "工作区存在未提交改动，请先提交或清理。" >&2
  exit 1
fi

git fetch origin main --tags

if [[ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]]; then
  echo "本地 main 与 origin/main 不一致，请先同步。" >&2
  exit 1
fi

if git show-ref --verify --quiet "refs/tags/$TAG"; then
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

echo "已发布 ${TAG}，GitHub Actions 将自动构建正式安装包。"
