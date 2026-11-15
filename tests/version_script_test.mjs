import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const workspace = fs.mkdtempSync(path.join(os.tmpdir(), 'translator-version-test-'));
const files = [
  'package.json',
  'Cargo.toml',
  'Cargo.lock',
  'android/app/build.gradle.kts',
  'packaging/windows-installer.nsi',
  'packaging/windows-portable-readme.txt',
];

try {
  for (const relative of files) {
    const destination = path.join(workspace, relative);
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    const crlf = fs.readFileSync(path.join(root, relative), 'utf8').replace(/\r?\n/g, '\r\n');
    fs.writeFileSync(destination, crlf);
  }

  const result = spawnSync(process.execPath, [path.join(root, 'scripts/set-version.mjs'), '7.8.9'], {
    cwd: workspace,
    encoding: 'utf8',
  });
  assert.equal(result.status, 0, result.stderr || result.stdout);
  assert.equal(JSON.parse(fs.readFileSync(path.join(workspace, 'package.json'), 'utf8')).version, '7.8.9');

  const nativeAndroid = fs.readFileSync(path.join(workspace, 'android/app/build.gradle.kts'), 'utf8');
  assert.match(nativeAndroid, /versionCode\s*=\s*7008009/);
  assert.match(nativeAndroid, /versionName\s*=\s*"7\.8\.9"/);
  assert.match(fs.readFileSync(path.join(workspace, 'Cargo.toml'), 'utf8'), /\[workspace\.package\][\s\S]*?version\s*=\s*"7\.8\.9"/);

  const lock = fs.readFileSync(path.join(workspace, 'Cargo.lock'), 'utf8');
  for (const crate of ['caption-platform', 'desktop-lifecycle', 'macos-system-audio', 'translator-core', 'translator-desktop']) {
    assert.match(lock, new RegExp(`\\[\\[package\\]\\]\\r?\\nname = "${crate}"\\r?\\nversion = "7\\.8\\.9"`));
  }
  assert.match(fs.readFileSync(path.join(workspace, 'packaging/windows-installer.nsi'), 'utf8'), /PRODUCT_VERSION "7\.8\.9"/);
  assert.match(fs.readFileSync(path.join(workspace, 'packaging/windows-portable-readme.txt'), 'utf8'), /^Gemini Live Translator 7\.8\.9/m);
  console.log('Version script checks passed for CRLF Windows checkouts.');
} finally {
  fs.rmSync(workspace, { recursive: true, force: true });
}
