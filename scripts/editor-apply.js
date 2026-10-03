#!/usr/bin/env node
// Applies the edits of the content editor (the `edits` collection of its artifact db) to
// data/*.ron with the editor's own RON code, so the files come out as the page shows them.
//
//   node scripts/editor-apply.js edits.json [--force]
//
// edits.json: the documents as ArtifactData lists them, an array of {id, data} (or of the
// documents themselves) or an object id -> document. Edits go in the order of `at`. A `set`
// whose record no longer reads as its `before` (data changed on main since the page was
// published) is a conflict and is skipped unless --force. Prints what was applied and what
// was not; exits 1 if anything was not.
'use strict';
const fs = require('fs');
const path = require('path');

const root = path.join(__dirname, '..');
const page = fs.readFileSync(path.join(root, 'editor/index.html'), 'utf8');
const section = name => {
  const s = page.indexOf('// ---------------------------------------------------------------- ' + name);
  return page.slice(s, page.indexOf('\n// ----------------', s + 1));
};
const fn = name => {
  const s = page.indexOf('function ' + name + '(');
  return page.slice(s, page.indexOf('\n}\n', s) + 3);
};
const R = new Function(
  [section('RON'), fn('applyEdit'),
    'return { parseRon, resolve, applyEdit, lineIndent, dedent };'].join('\n'))();

const [file, flag] = process.argv.slice(2);
if (!file) { console.error('usage: node scripts/editor-apply.js edits.json [--force]'); process.exit(2); }
const raw = JSON.parse(fs.readFileSync(file, 'utf8'));
const docs = (Array.isArray(raw) ? raw.map(d => [d.id, d.data || d]) : Object.entries(raw))
  .sort((a, b) => (a[1].at || '').localeCompare(b[1].at || ''));

const text = {};
const read = f => (text[f] ??= fs.readFileSync(path.join(root, 'data', f), 'utf8'));
let bad = 0;
for (const [id, e] of docs) {
  const where = `${id} ${e.op} ${e.file} ${e.path.join(' / ')} (${e.label || ''})`;
  let ast;
  try { ast = R.parseRon(read(e.file)); } catch (err) { console.log('НЕ ЧИТАЕТСЯ', where, err.message); bad++; continue; }
  if (e.op === 'set' && e.before != null && flag !== '--force') {
    const n = R.resolve(ast, e.path);
    const now = n && R.dedent(text[e.file].slice(n.s, n.e), R.lineIndent(text[e.file], n.s));
    if (now !== e.before) { console.log('КОНФЛИКТ', where, n ? 'запись изменилась на main' : 'записи нет'); bad++; continue; }
  }
  const t = R.applyEdit(text[e.file], ast, e);
  let ok = t != null;
  if (ok) try { R.parseRon(t); } catch { ok = false; }
  if (!ok) { console.log('НЕ ПРИМЕНЕНА', where); bad++; continue; }
  text[e.file] = t;
  console.log('применена', where);
}
for (const [f, t] of Object.entries(text)) fs.writeFileSync(path.join(root, 'data', f), t);
console.log(`правок ${docs.length}, применено ${docs.length - bad}, файлов ${Object.keys(text).length}`);
process.exit(bad ? 1 : 0);
