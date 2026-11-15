import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const workspace = fs.mkdtempSync(path.join(os.tmpdir(), 'translator-evidence-test-'));
const scopes = ['windows-macos', 'windows-macos', 'windows-macos', 'windows-macos', 'windows', 'windows-macos', 'windows-macos', 'macos'];
try {
  fs.mkdirSync(path.join(workspace, 'scripts'), { recursive: true });
  fs.mkdirSync(path.join(workspace, 'docs/assurance/evidence'), { recursive: true });
  fs.copyFileSync(path.join(root, 'scripts/check-target-evidence.mjs'), path.join(workspace, 'scripts/check-target-evidence.mjs'));
  fs.copyFileSync(path.join(root, 'scripts/version-utils.mjs'), path.join(workspace, 'scripts/version-utils.mjs'));
  const run = (...args) => spawnSync(process.execPath, ['scripts/check-target-evidence.mjs', ...args], { cwd: workspace, encoding: 'utf8' });
  assert.equal(run().status, 0, 'non-release validation reports missing evidence without fabricating it');
  assert.notEqual(run('--require-complete').status, 0, 'release validation must fail while evidence is absent');

  for (let index = 0; index < scopes.length; index += 1) {
    const number = String(index + 1).padStart(3, '0');
    fs.writeFileSync(path.join(workspace, `docs/assurance/evidence/target-${number}.json`), JSON.stringify({
      targetId: `ASSURANCE-TARGET-${number}`,
      scope: scopes[index],
      commit: 'a'.repeat(40),
      version: '1.1.0',
      executedOn: '2026-11-15',
      tester: 'tester-a',
      reviewer: 'reviewer-b',
      result: 'passed',
      cases: ['controlled physical-target case'],
      artifacts: [{ name: `target-${number}.zip`, sha256: 'b'.repeat(64) }],
    }));
  }
  assert.equal(run('--require-complete', '--commit', 'a'.repeat(40), '--version', '1.1.0').status, 0, 'complete independently reviewed evidence must pass for its exact release commit and version');
  assert.notEqual(run('--require-complete', '--commit', 'c'.repeat(40), '--version', '1.1.0').status, 0, 'evidence for another commit must be rejected');
  assert.notEqual(run('--require-complete', '--version', '1.2.0').status, 0, 'evidence for another final version must be rejected');
  assert.notEqual(run('--require-complete', '--version', '1.1.0-rc.1').status, 0, 'prerelease versions cannot be final evidence');

  const firstPath = path.join(workspace, 'docs/assurance/evidence/target-001.json');
  const first = JSON.parse(fs.readFileSync(firstPath, 'utf8'));
  first.reviewer = first.tester;
  fs.writeFileSync(firstPath, JSON.stringify(first));
  assert.notEqual(run('--require-complete').status, 0, 'self-review must be rejected');
  first.reviewer = 'reviewer-b';
  first.artifacts[0].sha256 = 'not-a-digest';
  fs.writeFileSync(firstPath, JSON.stringify(first));
  assert.notEqual(run('--require-complete').status, 0, 'malformed artifact identity must be rejected');
  first.artifacts[0].sha256 = 'b'.repeat(64);
  fs.writeFileSync(firstPath, JSON.stringify(first));
  fs.writeFileSync(path.join(workspace, 'docs/assurance/evidence/duplicate.json'), JSON.stringify(first));
  assert.notEqual(run('--require-complete').status, 0, 'duplicate target records must be rejected');

  console.log('Target evidence release-gate tests passed.');
} finally {
  fs.rmSync(workspace, { recursive: true, force: true });
}
