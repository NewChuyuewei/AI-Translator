import fs from 'node:fs';

const version = process.argv[2];
if (!version || !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)) {
  console.error('Usage: node scripts/set-version.mjs <major.minor.patch>');
  process.exit(2);
}

function json(path, update) {
  const value = JSON.parse(fs.readFileSync(path, 'utf8'));
  update(value);
  fs.writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}
function replace(path, pattern, replacement, label) {
  const current = fs.readFileSync(path, 'utf8');
  if (!pattern.test(current)) throw new Error(`Could not find ${label} in ${path}`);
  fs.writeFileSync(path, current.replace(pattern, replacement));
}

json('package.json', value => { value.version = version; });
json('package-lock.json', value => {
  value.version = version;
  value.packages[''].version = version;
});
replace('src-tauri/tauri.conf.json', /(\"version\"\s*:\s*\")[^\"]+(\")/, `$1${version}$2`, 'Tauri version');
replace('src-tauri/tauri.android.conf.json', /(\"version\"\s*:\s*\")[^\"]+(\")/, `$1${version}$2`, 'Android Tauri version');
replace('src-tauri/Cargo.toml', /(\[package\][\s\S]*?\nversion = ")[^"]+("\n)/, `$1${version}$2`, 'Cargo package version');
replace('src-tauri/Cargo.lock', /(\[\[package\]\]\nname = "gemini-live-translator"\nversion = ")[^"]+("\n)/, `$1${version}$2`, 'Cargo lock application version');
replace('packaging/windows-installer.nsi', /!define PRODUCT_VERSION "[^"]+"/, `!define PRODUCT_VERSION "${version}"`, 'NSIS product version');
replace('packaging/windows-portable-readme.txt', /^Gemini Live Translator [^ ]+/m, `Gemini Live Translator ${version}`, 'portable package version');

console.log(`Application build version set to ${version}`);
