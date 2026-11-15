import fs from 'node:fs';

const main = fs.readFileSync('desktop-slint/src/main.rs', 'utf8');
const ui = fs.readFileSync('desktop-slint/ui/app.slint', 'utf8');
const ci = fs.readFileSync('.github/workflows/ci.yml', 'utf8');
const release = fs.readFileSync('.github/workflows/release.yml', 'utf8');

const requiredMain = [
  ['Windows GUI subsystem', /cfg_attr\(target_os = "windows", windows_subsystem = "windows"\)/],
  ['runtime package version', /set_build_version\(env!\("CARGO_PKG_VERSION"\)/],
  ['session-scoped caption visibility', /caption_visible = Rc::new\(Cell::new\(false\)\)/],
];
for (const [label, pattern] of requiredMain) {
  if (!pattern.test(main)) throw new Error(`Missing desktop UX invariant: ${label}`);
}
if (/\bcaption\.show\(\)\?;/.test(main)) {
  throw new Error('Caption must not be shown unconditionally during startup');
}

const requiredUi = [
  ['forced light widget palette', /Palette\.color-scheme\s*=\s*ColorScheme\.light/],
  ['bounded scrolling control surface', /ScrollView\s*\{/],
  ['production window title', /title:\s*"Gemini Live Translator"/],
  ['dynamic build version', /root\.build-version/],
];
for (const [label, pattern] of requiredUi) {
  if (!pattern.test(ui)) throw new Error(`Missing desktop UI invariant: ${label}`);
}
if (/迁移预览|migration build/.test(ui)) throw new Error('Preview labeling must not remain in the production UI');
if (!fs.existsSync('scripts/verify-windows-gui.ps1')) throw new Error('Missing PE subsystem verifier');
for (const [label, workflow] of [['CI', ci], ['release', release]]) {
  if (!/verify-windows-gui\.ps1/.test(workflow)) throw new Error(`${label} does not verify the Windows GUI subsystem`);
}

console.log('Desktop UX boundary verified: no console, coherent light palette, scrolling controls, and session-scoped captions.');
