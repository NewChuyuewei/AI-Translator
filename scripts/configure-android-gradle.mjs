import fs from 'node:fs';
import path from 'node:path';

const propertiesPath = process.argv[2] || 'src-tauri/gen/android/gradle.properties';
const current = fs.readFileSync(propertiesPath, 'utf8');
const newline = current.includes('\r\n') ? '\r\n' : '\n';
const suppressions = new Set([
  'UNSUPPORTED_PROJECT_OPTION_USE',
  'DEPRECATED_DSL',
  'JAVA_COMPILE_DEPRECATED_SOURCE_TARGET',
]);
const output = [];

for (const line of current.split(/\r?\n/)) {
  if (line.startsWith('android.proguard.failOnMissingFiles=')) {
    // AGP's secure default is true. Keeping the old false override both hides
    // packaging errors and is unsupported in AGP 10.
    continue;
  }
  if (line.startsWith('android.javaCompile.suppressSourceTargetDeprecationWarning=')) {
    continue;
  }
  if (line.startsWith('android.sync.suppressAgpWarnings=')) {
    for (const value of line.slice(line.indexOf('=') + 1).split(',')) {
      if (value.trim()) suppressions.add(value.trim());
    }
    continue;
  }
  output.push(line);
}

while (output.at(-1) === '') output.pop();
output.push(
  '',
  '# Tauri 2 currently requires AGP legacy Kotlin/new-DSL compatibility flags.',
  '# Suppress only those known upstream migration warnings; do not hide other warnings.',
  'android.javaCompile.suppressSourceTargetDeprecationWarning=true',
  `android.sync.suppressAgpWarnings=${[...suppressions].sort().join(',')}`,
  '',
);
fs.writeFileSync(propertiesPath, output.join(newline));

const androidRoot = path.dirname(path.resolve(propertiesPath));
const appBuild = path.join(androidRoot, 'app', 'build.gradle.kts');
if (fs.existsSync(appBuild)) {
  const buildScript = fs.readFileSync(appBuild, 'utf8');
  let modern = buildScript.replace(
    /^android\s*\{/m,
    'extensions.configure<com.android.build.api.dsl.ApplicationExtension> {',
  );
  if (modern === buildScript && !buildScript.includes('extensions.configure<com.android.build.api.dsl.ApplicationExtension>')) {
    throw new Error(`Could not find generated Android application DSL in ${appBuild}`);
  }
  modern = modern
    .replaceAll('JavaVersion.VERSION_1_8', 'JavaVersion.VERSION_25')
    .replace(/\n\s*kotlinOptions\s*\{\s*jvmTarget\s*=\s*"(?:1\.8|17)"\s*\}/g, '')
    .replace(/\n\s*kotlinOptions\.jvmTarget\s*=\s*"(?:1\.8|17)"/g, '');
  if (!modern.includes('JvmTarget.JVM_25')) {
    modern += `${newline}${newline}// Project policy: compile generated application Kotlin to Java 25 bytecode.${newline}` +
      `tasks.withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile>().configureEach {${newline}` +
      `    compilerOptions {${newline}` +
      `        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_25)${newline}` +
      `    }${newline}` +
      `}${newline}`;
  }
  fs.writeFileSync(appBuild, modern);
}

// These Kotlin files are emitted by the current Tauri template. Their legacy
// Android calls are upstream compatibility paths, not application code. Keep
// suppression file-scoped so warnings in our Kotlin plugin stay visible.
for (const relative of [
  'app/src/main/java/com/gemini/live/translator/generated/RustWebView.kt',
  'app/src/main/java/com/gemini/live/translator/generated/WryActivity.kt',
]) {
  const generated = path.join(androidRoot, relative);
  if (!fs.existsSync(generated)) continue;
  const source = fs.readFileSync(generated, 'utf8');
  if (!source.startsWith('@file:Suppress("DEPRECATION")')) {
    fs.writeFileSync(generated, `@file:Suppress("DEPRECATION")${newline}${newline}${source}`);
  }
}

console.log(`Hardened generated Android Gradle project beside: ${propertiesPath}`);
