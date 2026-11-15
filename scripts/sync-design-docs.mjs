import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

// All paths are repository-relative; invocation works from any directory.
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const read = file => fs.readFileSync(path.join(root, file), 'utf8');
const normalize = text => text.replace(/\r\n/g, '\n').trimEnd() + '\n';
const names = fs.readdirSync(path.join(root, 'docs')).filter(n => /^\d\d-.*\.md$/.test(n)).sort();
const check = process.argv.includes('--check');
if (process.argv.slice(2).some(arg => arg !== '--check')) throw new Error('Usage: node scripts/sync-design-docs.mjs [--check]');
const errors = [];
const pairs = [['README.md', 'README.en.md'], ['docs/README.md', 'docs/en/README.md'], ...names.map(n => ['docs/' + n, 'docs/en/' + n])];
const markdown = pairs.flat();
const ids = text => [...new Set(text.match(/\b(?:REQ-[FN]-\d{3}|ADR-\d{3}|TASK-\d{3}[AB]?|TEST-[A-Z]+-\d{3})\b/g) || [])].sort();
for (const [ko, en] of pairs) {
  if (!fs.existsSync(path.join(root, en))) { errors.push('Missing counterpart: ' + en); continue; }
  const a = ids(read(ko)); const b = ids(read(en));
  if (JSON.stringify(a) !== JSON.stringify(b)) errors.push('ID mismatch: ' + ko + ' / ' + en + ' (' + a.filter(x => !b.includes(x)).join(',') + ' | ' + b.filter(x => !a.includes(x)).join(',') + ')');
}
function validate(file, text) {
  if ((text.match(/^```/gm) || []).length % 2) errors.push('Unbalanced code fence: ' + file);
  if (/@@BLOCK:\d+@@/.test(text)) errors.push('Unresolved translation block: ' + file);
  const prose = text.replace(/^```[^\n]*\n[\s\S]*?^```[^\n]*$/gm, '');
  for (const match of prose.matchAll(/\]\(([^\s)]+)\)/g)) {
    const target = match[1];
    if (/^(?:[a-z][a-z0-9+.-]*:|#)/i.test(target)) continue;
    const resolved = path.resolve(root, path.dirname(file), target.split('#')[0]);
    if (!fs.existsSync(resolved)) errors.push('Missing link: ' + file + ' -> ' + target);
  }
}
for (const file of markdown) if (fs.existsSync(path.join(root, file))) validate(file, read(file));
if (errors.length) { console.error(errors.join('\n')); process.exit(1); }

const schemas = ['schemas/0001_initial.sql', 'schemas/config.example.toml'];
function rebaseLinks(text, source) {
  let fenced = false;
  return text.split('\n').map(line => {
    if (line.startsWith('```')) { fenced = !fenced; return line; }
    if (fenced) return line;
    return line.replace(/\]\(([^\s)]+)\)/g, (whole, target) => {
      if (/^(?:[a-z][a-z0-9+.-]*:|#)/i.test(target)) return whole;
      return '](' + path.posix.normalize(path.posix.join(path.posix.dirname(source), target)) + ')';
    });
  }).join('\n');
}
function integrated(lang) {
  const en = lang === 'en';
  const sources = [en ? 'README.en.md' : 'README.md', ...names.map(n => (en ? 'docs/en/' : 'docs/') + n)];
  const title = en ? '# RPG World Simulation Engine — Complete Design' : '# RPG World Simulation Engine — 통합 설계서';
  const notice = en ? '> Generated from the English README/docs and shared schemas. Do not edit directly.' : '> 한국어 README/docs 및 공통 schemas에서 자동 생성합니다. 직접 편집하지 마세요.';
  const chunks = sources.map(file => '<!-- source: ' + file + ' -->\n\n' + rebaseLinks(normalize(read(file)).trimEnd(), file));
  for (const file of schemas) chunks.push('<!-- source: ' + file + ' -->\n\n```' + (file.endsWith('.sql') ? 'sql' : 'toml') + '\n' + normalize(read(file)) + '```');
  return [title, notice, ...chunks].join('\n\n---\n\n') + '\n';
}
const outputs = {
  'RPG_WSE_Complete_Design.md': integrated('ko'),
  'RPG_WSE_Complete_Design.en.md': integrated('en'),
};
for (const [file, text] of Object.entries(outputs)) validate(file, text);
if (errors.length) { console.error(errors.join('\n')); process.exit(1); }
const previous = JSON.parse(read('manifest.json'));
const manifest = {
  title: previous.title,
  baseline_date: previous.baseline_date,
  docs: names.length,
  languages: { ko: { readme: 'README.md', docs: 'docs' }, en: { readme: 'README.en.md', docs: 'docs/en' } },
  generator: 'scripts/sync-design-docs.mjs',
  files: {},
};
for (const file of [...markdown, ...schemas, ...Object.keys(outputs)].sort()) {
  const data = Buffer.from(normalize(outputs[file] ?? read(file)), 'utf8');
  manifest.files[file] = { sha256: crypto.createHash('sha256').update(data).digest('hex'), bytes: data.length };
}
// Normalize LF before hashing to remain reproducible with Git autocrlf on Windows.
manifest.hash_normalization = 'UTF-8; LF line endings; one final newline';
outputs['manifest.json'] = JSON.stringify(manifest, null, 2) + '\n';
for (const [file, text] of Object.entries(outputs)) {
  if (check) {
    if (!fs.existsSync(path.join(root, file)) || normalize(read(file)) !== normalize(text)) errors.push('Generated file is stale: ' + file);
  } else fs.writeFileSync(path.join(root, file), text, 'utf8');
}
if (errors.length) { console.error(errors.join('\n')); process.exit(1); }
console.log(`${check ? 'Checked' : 'Generated'} bilingual documentation: ${names.length} document pairs; links, fences, and IDs OK.`);
