import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'translator-gradle-test-'));
const properties = path.join(directory, 'gradle.properties');
const appBuild = path.join(directory, 'app', 'build.gradle.kts');
const generatedDirectory = path.join(directory, 'app/src/main/java/com/gemini/live/translator/generated');
const rustWebView = path.join(generatedDirectory, 'RustWebView.kt');
const wryActivity = path.join(generatedDirectory, 'WryActivity.kt');

try {
  fs.mkdirSync(path.dirname(appBuild), { recursive: true });
  fs.mkdirSync(generatedDirectory, { recursive: true });
  fs.writeFileSync(rustWebView, 'package com.gemini.live.translator.generated\r\n');
  fs.writeFileSync(wryActivity, 'package com.gemini.live.translator.generated\r\n');
  fs.writeFileSync(appBuild, [
    'plugins { id("com.android.application") }',
    '',
    'android {',
    '    namespace = "test"',
    '    compileOptions {',
    '        sourceCompatibility = JavaVersion.VERSION_1_8',
    '        targetCompatibility = JavaVersion.VERSION_1_8',
    '    }',
    '    kotlinOptions { jvmTarget = "1.8" }',
    '}',
    '',
  ].join('\r\n'));
  fs.writeFileSync(properties, [
    'org.gradle.jvmargs=-Xmx2048m',
    'android.proguard.failOnMissingFiles=false',
    'android.builtInKotlin=false',
    'android.newDsl=false',
    'android.javaCompile.suppressSourceTargetDeprecationWarning=false',
    'android.sync.suppressAgpWarnings=EXISTING_WARNING',
    '',
  ].join('\r\n'));

  const result = spawnSync(process.execPath, [path.join(root, 'scripts/configure-android-gradle.mjs'), properties], {
    encoding: 'utf8',
  });
  assert.equal(result.status, 0, result.stderr || result.stdout);

  const configured = fs.readFileSync(properties, 'utf8');
  assert.ok(configured.includes('\r\n'), 'must preserve Windows line endings');
  assert.doesNotMatch(configured, /^android\.proguard\.failOnMissingFiles=/m);
  assert.match(configured, /^android\.builtInKotlin=false$/m);
  assert.match(configured, /^android\.newDsl=false$/m);
  assert.match(configured, /^android\.javaCompile\.suppressSourceTargetDeprecationWarning=true$/m);
  assert.match(configured, /^android\.sync\.suppressAgpWarnings=DEPRECATED_DSL,EXISTING_WARNING,JAVA_COMPILE_DEPRECATED_SOURCE_TARGET,UNSUPPORTED_PROJECT_OPTION_USE$/m);
  const appScript = fs.readFileSync(appBuild, 'utf8');
  assert.doesNotMatch(appScript, /^android\s*\{/m);
  assert.match(appScript, /^extensions\.configure<com\.android\.build\.api\.dsl\.ApplicationExtension>\s*\{/m);
  assert.doesNotMatch(appScript, /JavaVersion\.VERSION_1_8|kotlinOptions/);
  assert.equal((appScript.match(/JavaVersion\.VERSION_25/g) || []).length, 2);
  assert.match(appScript, /JvmTarget\.JVM_25/);
  assert.match(fs.readFileSync(rustWebView, 'utf8'), /^@file:Suppress\("DEPRECATION"\)\r?\n\r?\npackage /);
  assert.match(fs.readFileSync(wryActivity, 'utf8'), /^@file:Suppress\("DEPRECATION"\)\r?\n\r?\npackage /);

  console.log('Generated Android Gradle property checks passed.');
} finally {
  fs.rmSync(directory, { recursive: true, force: true });
}
