import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { createInterface } from 'node:readline/promises';
import { fileURLToPath } from 'node:url';

process.chdir(resolve(dirname(fileURLToPath(import.meta.url)), '..'));

const versionPattern = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const jsonFiles = ['package.json', 'apps/desktop/package.json', 'src-tauri/tauri.conf.json'];
const cargoFile = 'src-tauri/Cargo.toml';

function run(command, args, { capture = false, shell = false } = {}) {
  const result = spawnSync(command, args, {
    encoding: 'utf8',
    shell,
    stdio: capture ? ['ignore', 'pipe', 'pipe'] : 'inherit',
  });

  if (result.error) {
    throw new Error(`无法执行 ${command}：${result.error.message}`);
  }
  if (result.status !== 0) {
    if (capture && result.stderr) process.stderr.write(result.stderr);
    throw new Error(`命令执行失败：${command} ${args.join(' ')}`);
  }

  return capture ? result.stdout.trim() : '';
}

function runPnpm(args) {
  if (process.env.npm_execpath) {
    return run(process.execPath, [process.env.npm_execpath, ...args]);
  }
  return run('pnpm', args, { shell: process.platform === 'win32' });
}

function runChecks() {
  runPnpm(['--filter', '@cortana/desktop', 'exec', 'oxfmt', '--check', '.']);
  runPnpm(['--filter', '@cortana/desktop', 'exec', 'oxlint', 'src']);
  runPnpm(['--filter', '@cortana/desktop', 'exec', 'vitest', 'run']);
  runPnpm(['--filter', '@cortana/desktop', 'build:web']);
  run('cargo', ['fmt', '--manifest-path', 'src-tauri/Cargo.toml', '--', '--check']);
  run('cargo', ['clippy', '--manifest-path', 'src-tauri/Cargo.toml', '--all-targets', '--', '-D', 'warnings']);
  run('cargo', ['test', '--manifest-path', 'src-tauri/Cargo.toml']);
}

function versionParts(version) {
  if (typeof version !== 'string') return null;
  const match = version.match(versionPattern);
  return match?.slice(1).map(BigInt);
}

function compareVersions(left, right) {
  const a = versionParts(left);
  const b = versionParts(right);
  if (!a || !b) throw new Error('无法比较无效版本号。');

  for (let index = 0; index < 3; index += 1) {
    if (a[index] !== b[index]) return a[index] > b[index] ? 1 : -1;
  }
  return 0;
}

function nextVersions(version) {
  const [major, minor, patch] = versionParts(version);
  return [`${major}.${minor}.${patch + 1n}`, `${major}.${minor + 1n}.0`, `${major + 1n}.0.0`];
}

// ponytail: one inline check is enough for release-only version logic.
assert.deepEqual(nextVersions('1.2.3'), ['1.2.4', '1.3.0', '2.0.0']);
assert.equal(compareVersions('1.10.0', '1.9.9'), 1);

function readVersions() {
  const jsonDocuments = jsonFiles.map((file) => [file, JSON.parse(readFileSync(file, 'utf8'))]);
  const cargo = readFileSync(cargoFile, 'utf8');
  const cargoMatch = cargo.match(/^version = "([^"]+)"$/m);

  if (!cargoMatch) throw new Error('无法读取 Cargo.toml 版本号。');

  const versions = [...jsonDocuments.map(([, document]) => document.version), cargoMatch[1]];
  if (new Set(versions).size !== 1) {
    throw new Error(`当前版本号不一致：${versions.join(', ')}`);
  }
  if (!versionParts(versions[0])) {
    throw new Error(`当前版本号格式错误：${versions[0]}`);
  }

  return { cargo, cargoMatch, currentVersion: versions[0], jsonDocuments };
}

async function chooseVersion(currentVersion) {
  if (!process.stdin.isTTY) {
    throw new Error('非交互环境请使用 pnpm release <版本号>。');
  }

  const choices = [...nextVersions(currentVersion), null];
  const labels = [
    `补丁版本 Patch：${choices[0]}`,
    `次版本 Minor：${choices[1]}`,
    `主版本 Major：${choices[2]}`,
    '取消发布',
  ];
  const readline = createInterface({
    input: process.stdin,
    output: process.stdout,
  });

  try {
    console.log(`当前版本：${currentVersion}`);
    labels.forEach((label, index) => console.log(`${index + 1}) ${label}`));

    let selection;
    while (!selection) {
      const answer = (await readline.question('请选择 [1-4]：')).trim();
      if (/^[1-4]$/.test(answer)) selection = Number(answer);
      else console.error('请输入 1-4。');
    }

    const version = choices[selection - 1];
    if (!version) return null;

    const confirm = await readline.question(`确认发布 v${version}？[y/N] `);
    return /^[Yy]$/.test(confirm.trim()) ? version : null;
  } finally {
    readline.close();
  }
}

async function main() {
  const args = process.argv.slice(2);
  if (args.length > 1) {
    throw new Error('用法：pnpm release [版本号]');
  }

  const state = readVersions();
  if (args[0] && !versionParts(args[0])) {
    throw new Error('版本号格式错误，应为 X.Y.Z。');
  }

  if (run('git', ['branch', '--show-current'], { capture: true }) !== 'main') {
    throw new Error('发布只能在 main 分支执行。');
  }
  if (run('git', ['status', '--porcelain'], { capture: true })) {
    throw new Error('工作区存在未提交改动，请先提交或清理。');
  }

  run('git', ['fetch', 'origin', 'main']);
  if (
    run('git', ['rev-parse', 'HEAD'], { capture: true }) !==
    run('git', ['rev-parse', 'origin/main'], { capture: true })
  ) {
    throw new Error('本地 main 与 origin/main 不一致，请先同步。');
  }

  runChecks();

  const version = args[0] || (await chooseVersion(state.currentVersion));
  if (!version) {
    console.log('已取消发布。');
    return;
  }
  if (compareVersions(version, state.currentVersion) <= 0) {
    throw new Error(`新版本 ${version} 必须高于当前版本 ${state.currentVersion}。`);
  }

  const tag = `v${version}`;
  if (
    run('git', ['tag', '--list', tag], { capture: true }) ||
    run('git', ['ls-remote', '--tags', 'origin', `refs/tags/${tag}`], {
      capture: true,
    })
  ) {
    throw new Error(`Tag ${tag} 已存在。`);
  }

  for (const [file, document] of state.jsonDocuments) {
    document.version = version;
    writeFileSync(file, `${JSON.stringify(document, null, 2)}\n`);
  }
  writeFileSync(cargoFile, state.cargo.replace(state.cargoMatch[0], `version = "${version}"`));

  run('git', [
    'add',
    'package.json',
    'apps/desktop/package.json',
    'src-tauri/Cargo.toml',
    'src-tauri/Cargo.lock',
    'src-tauri/tauri.conf.json',
  ]);
  run('git', ['commit', '-m', `chore(release): ${tag}`]);
  run('git', ['tag', '-a', tag, '-m', `Cortana ${tag}`]);
  run('git', ['push', '--atomic', 'origin', 'main', tag]);

  console.log(`已推送 ${tag}，GitHub Actions 将创建 Release 并上传正式安装包。`);
}

try {
  await main();
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
}
