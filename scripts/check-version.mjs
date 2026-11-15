import fs from 'node:fs';
import { parseVersion } from './version-utils.mjs';

const packageVersion = JSON.parse(fs.readFileSync('package.json', 'utf8')).version;
const parsedVersion = parseVersion(packageVersion);
const found = new Map([['release metadata', packageVersion]]);
const capture = (name, path, pattern) => {
  const match = fs.readFileSync(path, 'utf8').match(pattern);
  if (!match) throw new Error(`Could not read ${name} version from ${path}`);
  found.set(name, match[1]);
};

capture('Cargo workspace / Slint desktop', 'Cargo.toml', /\[workspace\.package\][\s\S]*?\nversion\s*=\s*"([^"]+)"/);
capture('native Android', 'android/app/build.gradle.kts', /versionName\s*=\s*"([^"]+)"/);
capture('Windows installer', 'packaging/windows-installer.nsi', /PRODUCT_VERSION "([^"]+)"/);
const installer = fs.readFileSync('packaging/windows-installer.nsi', 'utf8');
const fileVersion = installer.match(/PRODUCT_FILE_VERSION "([^"]+)"/)?.[1];
if (fileVersion !== parsedVersion.fileVersion) {
  throw new Error(`Windows numeric file version mismatch: expected ${parsedVersion.fileVersion}, got ${fileVersion || 'missing'}`);
}
capture('Windows portable readme', 'packaging/windows-portable-readme.txt', /^Gemini Live Translator ([^ ]+)/m);

const workspaceCrates = [
  'caption-platform',
  'desktop-lifecycle',
  'macos-system-audio',
  'translator-core',
  'translator-desktop',
];
const lock = fs.readFileSync('Cargo.lock', 'utf8');
for (const crate of workspaceCrates) {
  const escaped = crate.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = lock.match(new RegExp(`\\[\\[package\\]\\]\\r?\\nname = "${escaped}"\\r?\\nversion = "([^"]+)"`));
  if (!match) throw new Error(`Could not read ${crate} version from Cargo.lock`);
  found.set(`Cargo.lock ${crate}`, match[1]);
}

const mismatches = [...found].filter(([, version]) => version !== packageVersion);
if (mismatches.length) {
  const details = [...found].map(([name, version]) => `${name}: ${version}`).join('\n');
  throw new Error(`Version mismatch; run node scripts/set-version.mjs ${packageVersion}\n${details}`);
}
console.log(`All application version sources agree on ${packageVersion}.`);
