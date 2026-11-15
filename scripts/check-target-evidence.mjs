import fs from 'node:fs';
import path from 'node:path';

const required = new Map([
  ['ASSURANCE-TARGET-001', 'windows-macos'],
  ['ASSURANCE-TARGET-002', 'windows-macos'],
  ['ASSURANCE-TARGET-003', 'windows-macos'],
  ['ASSURANCE-TARGET-004', 'windows-macos'],
  ['ASSURANCE-TARGET-005', 'windows'],
  ['ASSURANCE-TARGET-006', 'windows-macos'],
  ['ASSURANCE-TARGET-007', 'windows-macos'],
  ['ASSURANCE-TARGET-008', 'macos'],
]);
const directory = 'docs/assurance/evidence';
const requireComplete = process.argv.includes('--require-complete');
const commitIndex = process.argv.indexOf('--commit');
const expectedCommit = commitIndex >= 0 ? process.argv[commitIndex + 1] : undefined;
if (commitIndex >= 0 && !/^[0-9a-f]{40}$/.test(expectedCommit || '')) throw new Error('--commit requires a full lowercase Git SHA');
const records = new Map();

if (fs.existsSync(directory)) {
  for (const filename of fs.readdirSync(directory).filter(name => name.endsWith('.json')).sort()) {
    const record = JSON.parse(fs.readFileSync(path.join(directory, filename), 'utf8'));
    if (!required.has(record.targetId)) throw new Error(`${filename}: unknown targetId`);
    if (records.has(record.targetId)) throw new Error(`${filename}: duplicate ${record.targetId}`);
    if (record.scope !== required.get(record.targetId)) throw new Error(`${filename}: incorrect scope`);
    if (!/^[0-9a-f]{40}$/.test(record.commit || '')) throw new Error(`${filename}: commit must be a full Git SHA`);
    if (expectedCommit && record.commit !== expectedCommit) throw new Error(`${filename}: evidence is not for release commit ${expectedCommit}`);
    if (!/^\d{4}-\d{2}-\d{2}$/.test(record.executedOn || '')) throw new Error(`${filename}: invalid executedOn`);
    if (!record.tester || !record.reviewer || record.tester === record.reviewer) throw new Error(`${filename}: independent tester and reviewer are required`);
    if (record.result !== 'passed') throw new Error(`${filename}: result must be passed`);
    if (!Array.isArray(record.cases) || record.cases.length === 0 || record.cases.some(value => typeof value !== 'string' || !value.trim())) throw new Error(`${filename}: non-empty cases are required`);
    if (!Array.isArray(record.artifacts) || record.artifacts.length === 0 || record.artifacts.some(item => !item.name || !/^[0-9a-f]{64}$/.test(item.sha256 || ''))) throw new Error(`${filename}: artifact names and SHA-256 values are required`);
    records.set(record.targetId, filename);
  }
}

const missing = [...required.keys()].filter(id => !records.has(id));
if (requireComplete && missing.length) throw new Error(`Target validation is incomplete: ${missing.join(', ')}`);
const commits = new Set([...records.keys()].map(id => {
  const filename = records.get(id);
  return JSON.parse(fs.readFileSync(path.join(directory, filename), 'utf8')).commit;
}));
if (requireComplete && commits.size !== 1) throw new Error('All target evidence must identify one tested release commit');
if (process.argv.includes('--print-commit')) {
  if (!requireComplete || commits.size !== 1) throw new Error('--print-commit requires complete evidence for one commit');
  process.stdout.write(`${[...commits][0]}\n`);
} else {
  console.log(`Target evidence: ${records.size}/${required.size} complete.${missing.length ? ` Open: ${missing.join(', ')}` : ''}`);
}
