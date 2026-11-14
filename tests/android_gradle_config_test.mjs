import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'translator-gradle-test-'));
const properties = path.join(directory, 'gradle.properties');
const rootBuild = path.join(directory, 'build.gradle.kts');
const wrapperProperties = path.join(directory, 'gradle/wrapper/gradle-wrapper.properties');
const appBuild = path.join(directory, 'app', 'build.gradle.kts');
const generatedDirectory = path.join(directory, 'app/src/main/java/com/gemini/live/translator/generated');
const rustWebView = path.join(generatedDirectory, 'RustWebView.kt');
const wryActivity = path.join(generatedDirectory, 'WryActivity.kt');

try {
  fs.mkdirSync(path.dirname(appBuild), { recursive: true });
  fs.mkdirSync(path.dirname(wrapperProperties), { recursive: true });
  fs.mkdirSync(generatedDirectory, { recursive: true });
  fs.writeFileSync(rootBuild, 'buildscript { dependencies { classpath("com.android.tools.build:gradle:9.3.1"); classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:2.2.10") } }\r\n');
  fs.writeFileSync(wrapperProperties, 'distributionUrl=https\\://services.gradle.org/distributions/gradle-9.6.1-bin.zip\r\n');
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
    '}',
    '',
    'kotlin {',
    '    compilerOptions {',
    '        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_1_8',
    '    }',
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

  const firstPass = new Map([
    [properties, fs.readFileSync(properties, 'utf8')],
    [rootBuild, fs.readFileSync(rootBuild, 'utf8')],
    [wrapperProperties, fs.readFileSync(wrapperProperties, 'utf8')],
    [appBuild, fs.readFileSync(appBuild, 'utf8')],
  ]);
  const repeated = spawnSync(process.execPath, [path.join(root, 'scripts/configure-android-gradle.mjs'), properties], {
    encoding: 'utf8',
  });
  assert.equal(repeated.status, 0, repeated.stderr || repeated.stdout);
  for (const [file, content] of firstPass) {
    assert.equal(fs.readFileSync(file, 'utf8'), content, `${file} must be idempotent`);
  }

  const configured = fs.readFileSync(properties, 'utf8');
  assert.ok(configured.includes('\r\n'), 'must preserve Windows line endings');
  assert.doesNotMatch(configured, /^android\.proguard\.failOnMissingFiles=/m);
  assert.match(configured, /^android\.builtInKotlin=false$/m);
  assert.match(configured, /^android\.newDsl=false$/m);
  assert.match(configured, /^android\.javaCompile\.suppressSourceTargetDeprecationWarning=true$/m);
  assert.match(configured, /^kotlin\.jvm\.target\.validation\.mode=ignore$/m);
  assert.match(configured, /^kotlin\.compiler\.runViaBuildToolsApi=true$/m);
  assert.match(configured, /^android\.sync\.suppressAgpWarnings=DEPRECATED_DSL,EXISTING_WARNING,JAVA_COMPILE_DEPRECATED_SOURCE_TARGET,UNSUPPORTED_PROJECT_OPTION_USE$/m);
  const rootScript = fs.readFileSync(rootBuild, 'utf8');
  assert.match(rootScript, /com\.android\.tools\.build:gradle:9\.4\.1/);
  assert.match(rootScript, /kotlin-gradle-plugin:2\.4\.21/);
  assert.match(rootScript, /kotlin-gradle-plugin-api:2\.4\.21/);
  assert.match(rootScript, /kotlin-compiler-embeddable:2\.4\.21/);
  assert.match(fs.readFileSync(wrapperProperties, 'utf8'), /gradle-9\.9\.1-bin\.zip/);
  const appScript = fs.readFileSync(appBuild, 'utf8');
  assert.doesNotMatch(appScript, /^android\s*\{/m);
  assert.match(appScript, /^extensions\.configure<com\.android\.build\.api\.dsl\.ApplicationExtension>\s*\{/m);
  assert.doesNotMatch(appScript, /JavaVersion\.VERSION_(?:1_8|17)|JvmTarget\.JVM_|kotlinOptions/);
  assert.equal((appScript.match(/JavaVersion\.VERSION_25/g) || []).length, 2);
  assert.match(appScript, /freeCompilerArgs\.addAll\("-jvm-target", "25"\)/);
  assert.match(fs.readFileSync(rustWebView, 'utf8'), /^@file:Suppress\("DEPRECATION"\)\r?\n\r?\npackage /);
  assert.match(fs.readFileSync(wryActivity, 'utf8'), /^@file:Suppress\("DEPRECATION"\)\r?\n\r?\npackage /);

  console.log('Generated Android Gradle property checks passed.');
} finally {
  fs.rmSync(directory, { recursive: true, force: true });
}
