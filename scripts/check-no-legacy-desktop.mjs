import fs from 'node:fs';

const forbiddenPaths = ['src-tauri', 'src', 'vite.config.js', 'package-lock.json'];
const present = forbiddenPaths.filter(path => fs.existsSync(path));
if (present.length) throw new Error(`Legacy desktop paths must not exist: ${present.join(', ')}`);

const packageJson = JSON.parse(fs.readFileSync('package.json', 'utf8'));
const dependencyNames = [
  ...Object.keys(packageJson.dependencies || {}),
  ...Object.keys(packageJson.devDependencies || {}),
];
const forbiddenDependencies = dependencyNames.filter(name => name.includes('tauri') || name === 'vite' || name === 'jsdom');
if (forbiddenDependencies.length) throw new Error(`Legacy desktop dependencies remain: ${forbiddenDependencies.join(', ')}`);

for (const path of ['.github/workflows/ci.yml', '.github/workflows/release.yml', 'packaging/windows-installer.nsi']) {
  const text = fs.readFileSync(path, 'utf8');
  if (/src-tauri|WebView2|npm run tauri|npm run build|test:ui/.test(text)) {
    throw new Error(`Legacy desktop runtime reference remains in ${path}`);
  }
}
console.log('Pure Slint desktop boundary verified: no Tauri, WebView UI, Vite, or jsdom runtime path.');
