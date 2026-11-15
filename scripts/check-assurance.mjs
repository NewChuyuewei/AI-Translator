import fs from 'node:fs';

const requirementsPath = 'docs/assurance/SOFTWARE_REQUIREMENTS.md';
const tracePath = 'docs/assurance/TRACEABILITY.md';
const requirementsText = fs.readFileSync(requirementsPath, 'utf8');
const traceText = fs.readFileSync(tracePath, 'utf8');
const ids = [...new Set(requirementsText.match(/SWR-[A-Z]+-\d{3}/g) || [])].sort();
if (ids.length === 0) throw new Error('No software requirement IDs found');

const missing = ids.filter(id => !new RegExp(`^\\| ${id.replaceAll('-', '\\-')} \\|`, 'm').test(traceText));
if (missing.length) throw new Error(`Requirements missing from traceability matrix: ${missing.join(', ')}`);

const duplicateRows = ids.filter(id => (traceText.match(new RegExp(`^\\| ${id.replaceAll('-', '\\-')} \\|`, 'gm')) || []).length !== 1);
if (duplicateRows.length) throw new Error(`Requirements must have exactly one trace row: ${duplicateRows.join(', ')}`);

console.log(`Assurance traceability checks passed for ${ids.length} requirements.`);
