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
  'package-lock.json',
  'Cargo.toml',
  'src-tauri/tauri.conf.json',
  'src-tauri/tauri.android.conf.json',
  'android/app/build.gradle.kts',
  'src-tauri/Cargo.toml',
  'src-tauri/Cargo.lock',
  'packaging/windows-installer.nsi',
  'packaging/windows-portable-readme.txt',
];

try {
  for (const relative of files) {
    const destination = path.join(workspace, relative);
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    // Simulate actions/checkout on a Windows runner with core.autocrlf=true.
    const crlf = fs.readFileSync(path.join(root, relative), 'utf8').replace(/\r?\n/g, '\r\n');
    fs.writeFileSync(destination, crlf);
  }

  const result = spawnSync(process.execPath, [path.join(root, 'scripts/set-version.mjs'), '7.8.9'], {
    cwd: workspace,
    encoding: 'utf8',
  });
  assert.equal(result.status, 0, result.stderr || result.stdout);

  const packageJson = JSON.parse(fs.readFileSync(path.join(workspace, 'package.json'), 'utf8'));
  const packageLock = JSON.parse(fs.readFileSync(path.join(workspace, 'package-lock.json'), 'utf8'));
  const tauri = JSON.parse(fs.readFileSync(path.join(workspace, 'src-tauri/tauri.conf.json'), 'utf8'));
  const android = JSON.parse(fs.readFileSync(path.join(workspace, 'src-tauri/tauri.android.conf.json'), 'utf8'));
  assert.equal(packageJson.version, '7.8.9');
  assert.equal(packageLock.version, '7.8.9');
  assert.equal(packageLock.packages[''].version, '7.8.9');
  assert.equal(tauri.version, '7.8.9');
  assert.equal(android.version, '7.8.9');
  const nativeAndroid = fs.readFileSync(path.join(workspace, 'android/app/build.gradle.kts'), 'utf8');
  assert.match(nativeAndroid, /versionCode\s*=\s*7008009/);
  assert.match(nativeAndroid, /versionName\s*=\s*"7\.8\.9"/);

  const workspaceCargo = fs.readFileSync(path.join(workspace, 'Cargo.toml'), 'utf8');
  assert.match(workspaceCargo, /\[workspace\.package\][\s\S]*?version\s*=\s*"7\.8\.9"/);
  const cargoToml = fs.readFileSync(path.join(workspace, 'src-tauri/Cargo.toml'), 'utf8');
  const cargoLock = fs.readFileSync(path.join(workspace, 'src-tauri/Cargo.lock'), 'utf8');
  assert.match(cargoToml, /\[package\][\s\S]*?version\s*=\s*"7\.8\.9"/);
  assert.match(cargoLock, /\[\[package\]\][\r\n]+name\s*=\s*"gemini-live-translator"[\r\n]+version\s*=\s*"7\.8\.9"/);
  assert.match(fs.readFileSync(path.join(workspace, 'packaging/windows-installer.nsi'), 'utf8'), /PRODUCT_VERSION "7\.8\.9"/);
  assert.match(fs.readFileSync(path.join(workspace, 'packaging/windows-portable-readme.txt'), 'utf8'), /^Gemini Live Translator 7\.8\.9/m);

  console.log('Version script checks passed for CRLF Windows checkouts.');
} finally {
  fs.rmSync(workspace, { recursive: true, force: true });
}
