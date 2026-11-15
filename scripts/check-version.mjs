import fs from 'node:fs';

const packageVersion = JSON.parse(fs.readFileSync('package.json', 'utf8')).version;
const found = new Map();
const capture = (name, path, pattern) => {
  const match = fs.readFileSync(path, 'utf8').match(pattern);
  if (!match) throw new Error(`Could not read ${name} version from ${path}`);
  found.set(name, match[1]);
};

found.set('npm package', packageVersion);
found.set('npm lock', JSON.parse(fs.readFileSync('package-lock.json', 'utf8')).version);
found.set('Tauri config', JSON.parse(fs.readFileSync('src-tauri/tauri.conf.json', 'utf8')).version);
found.set('legacy Android config', JSON.parse(fs.readFileSync('src-tauri/tauri.android.conf.json', 'utf8')).version);
capture('Cargo workspace / Slint desktop', 'Cargo.toml', /\[workspace\.package\][\s\S]*?\nversion\s*=\s*"([^"]+)"/);
capture('legacy Tauri Cargo package', 'src-tauri/Cargo.toml', /\[package\][\s\S]*?\nversion\s*=\s*"([^"]+)"/);
capture('native Android', 'android/app/build.gradle.kts', /versionName\s*=\s*"([^"]+)"/);
capture('Windows installer', 'packaging/windows-installer.nsi', /PRODUCT_VERSION "([^"]+)"/);
capture('Windows portable readme', 'packaging/windows-portable-readme.txt', /^Gemini Live Translator ([^ ]+)/m);

const mismatches = [...found].filter(([, version]) => version !== packageVersion);
if (mismatches.length) {
  const details = [...found].map(([name, version]) => `${name}: ${version}`).join('\n');
  throw new Error(`Version mismatch; run node scripts/set-version.mjs ${packageVersion}\n${details}`);
}
console.log(`All application version sources agree on ${packageVersion}.`);
