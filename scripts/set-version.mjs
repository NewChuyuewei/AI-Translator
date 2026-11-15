import fs from 'node:fs';

const version = process.argv[2];
const match = version?.match(/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/);
if (!match) {
  console.error('Usage: node scripts/set-version.mjs <major.minor.patch>');
  process.exit(2);
}
const versionCode = Number(match[1]) * 1_000_000 + Number(match[2]) * 1_000 + Number(match[3]);
if (!Number.isSafeInteger(versionCode) || versionCode > 2_100_000_000) {
  throw new Error('Version is too large for Android versionCode');
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
replace('android/app/build.gradle.kts', /(versionCode\s*=\s*)\d+/, `$1${versionCode}`, 'native Android versionCode');
replace('android/app/build.gradle.kts', /(versionName\s*=\s*")[^"]+(")/, `$1${version}$2`, 'native Android versionName');
replace('Cargo.toml', /(\[workspace\.package\][\s\S]*?[\r\n]+version\s*=\s*")[^"]+(")/, `$1${version}$2`, 'workspace package version');
for (const crate of [
  'caption-platform',
  'desktop-lifecycle',
  'macos-system-audio',
  'translator-core',
  'translator-desktop',
]) {
  replace(
    'Cargo.lock',
    new RegExp(`(\\[\\[package\\]\\][\\r\\n]+name\\s*=\\s*"${crate}"[\\r\\n]+version\\s*=\\s*")[^"]+(")`),
    `$1${version}$2`,
    `${crate} workspace lock version`,
  );
}
replace('packaging/windows-installer.nsi', /!define PRODUCT_VERSION "[^"]+"/, `!define PRODUCT_VERSION "${version}"`, 'NSIS product version');
replace('packaging/windows-portable-readme.txt', /^Gemini Live Translator [^ ]+/m, `Gemini Live Translator ${version}`, 'portable package version');

console.log(`Application build version set to ${version}`);
