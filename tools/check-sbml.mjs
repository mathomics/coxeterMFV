#!/usr/bin/env node
/**
 * Validate the SBML this project writes against libSBML, the real validator.
 *
 *   npm run check:sbml
 *
 * The round-trip tests in test/sbml-write.test.mjs check that nothing is lost
 * between here and back, which is the part that catches dropped charges and
 * flipped signs. They cannot tell whether the file is *conformant*, because
 * they read it with the same reader that wrote it. Only libSBML can say that.
 *
 * libSBML is not a dependency of this project and is not needed to use it, so
 * this check skips with a message when it is absent:
 *
 *   pip install python-libsbml
 *
 * Warnings are reported but do not fail: a constraint-based model leaves
 * species amounts and most units undeclared, so every published FBC file,
 * including the ones this project reads, raises the same ones.
 */
import { readFileSync, readdirSync, existsSync, writeFileSync, mkdtempSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

import { Model } from '../src/core/model.js';
import { toSBML } from '../src/core/sbml-write.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

const VALIDATOR = `
import sys, json, collections
try:
    import libsbml
except ImportError:
    print(json.dumps({"missing": True}))
    sys.exit(0)

out = []
for path in sys.argv[1:]:
    doc = libsbml.readSBMLFromFile(path)
    read_errors = [doc.getError(i) for i in range(doc.getNumErrors())]
    fatal = [e.getMessage()[:200] for e in read_errors
             if e.getSeverityAsString() in ("Error", "Fatal")]
    doc.checkConsistency()
    problems = collections.Counter()
    details = []
    for i in range(doc.getNumErrors()):
        e = doc.getError(i)
        sev = e.getSeverityAsString()
        problems[sev] += 1
        if sev in ("Error", "Fatal"):
            details.append(f"{e.getErrorId()}: {e.getMessage()[:200]}")
    model = doc.getModel()
    plugin = model.getPlugin("fbc") if model else None
    out.append({
        "path": path,
        "readErrors": fatal,
        "errors": problems.get("Error", 0) + problems.get("Fatal", 0),
        "warnings": problems.get("Warning", 0),
        "details": details[:3],
        "level": doc.getLevel(), "version": doc.getVersion(),
        "fbc": None if plugin is None else plugin.getPackageVersion(),
        "strict": None if plugin is None else plugin.getStrict(),
        "reactions": 0 if model is None else model.getNumReactions(),
        "species": 0 if model is None else model.getNumSpecies(),
    })
print(json.dumps(out))
`;

const docs = [];
for (const file of readdirSync(join(root, 'models'))) {
  if (!file.endsWith('.json') || ['index.json', 'scenarios.json'].includes(file)) continue;
  docs.push({ label: `models/${file}`, path: join(root, 'models', file) });
}
const publishedDir = join(root, 'models', 'published');
if (existsSync(publishedDir)) {
  for (const dir of readdirSync(publishedDir)) {
    for (const file of readdirSync(join(publishedDir, dir))) {
      if (file.startsWith('model-')) {
        docs.push({ label: `${dir}/${file}`, path: join(publishedDir, dir, file) });
      }
    }
  }
}

const work = mkdtempSync(join(tmpdir(), 'coxeter-sbml-'));
const written = [];
for (const doc of docs) {
  const model = new Model(JSON.parse(readFileSync(doc.path, 'utf8')));
  const out = join(work, `${doc.label.replace(/[^\w.-]/g, '_')}.xml`);
  writeFileSync(out, toSBML(model));
  written.push({ ...doc, out });
}

const script = join(work, 'validate.py');
writeFileSync(script, VALIDATOR);

let raw;
try {
  raw = execFileSync('python3', [script, ...written.map((w) => w.out)],
    { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
} catch (error) {
  console.log('No se pudo ejecutar python3, así que libSBML no validó nada.');
  console.log(`  ${error.message.split('\n')[0]}`);
  console.log('  Los tests de ida y vuelta (npm test) sí corren sin esto.');
  process.exit(0);
}

const report = JSON.parse(raw);
if (report.missing) {
  console.log('libSBML no está instalado, así que no se validó nada contra el estándar.');
  console.log('  pip install python-libsbml');
  console.log('  Los tests de ida y vuelta (npm test) sí corren sin esto.');
  process.exit(0);
}

let failures = 0;
let warnings = 0;
for (const [i, result] of report.entries()) {
  const { label } = written[i];
  const bad = result.readErrors.length > 0 || result.errors > 0;
  if (bad) failures += 1;
  warnings += result.warnings;
  const shape = `L${result.level}V${result.version} fbc-v${result.fbc}`
    + `${result.strict ? ' strict' : ''}, ${result.reactions} rxn, ${result.species} met`;
  console.log(`${bad ? 'FALLA' : 'ok   '}  ${label.padEnd(42)} ${shape}`);
  for (const line of [...result.readErrors, ...result.details]) console.log(`        ${line}`);
}

console.log(`\n${report.length} archivos, ${failures} con errores, `
  + `${warnings} avisos (unidades y cantidades iniciales, los mismos que trae un archivo FBC publicado).`);
process.exit(failures ? 1 : 0);
