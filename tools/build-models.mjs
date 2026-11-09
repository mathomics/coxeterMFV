#!/usr/bin/env node
/**
 * Emit models/*.json from the compact species descriptions in tools/species/.
 * The JSON files are the canonical artefacts: they are plain COBRA documents
 * that load in COBRApy as well as in this app.
 *
 * Usage: node tools/build-models.mjs
 */
import { writeFile, mkdir } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');
const outDir = join(root, 'models');

const SPECIES = [
  { file: 'ecoli.mjs', out: 'ecoli_core_mc.json' },
  { file: 'yeast.mjs', out: 'yeast_core_mc.json' },
  { file: 'cyano.mjs', out: 'cyano_core_mc.json' },
  { file: 'methanogen.mjs', out: 'methanogen_core_mc.json' },
  { file: 'lactobacillus.mjs', out: 'lactobacillus_core_mc.json' },
];

await mkdir(outDir, { recursive: true });

const index = [];
for (const entry of SPECIES) {
  // import() needs a file:// URL. A bare absolute path is read as a URL
  // scheme on Windows ("c:").
  const mod = await import(pathToFileURL(join(here, 'species', entry.file)).href);
  const doc = mod.default();
  await writeFile(join(outDir, entry.out), `${JSON.stringify(doc, null, 2)}\n`, 'utf8');
  index.push({
    file: entry.out,
    id: doc.id,
    name: doc.name,
    cell: doc.cell,
    compartments: doc.compartments,
    counts: { metabolites: doc.metabolites.length, reactions: doc.reactions.length },
  });
  console.log(`wrote models/${entry.out}  (${doc.metabolites.length} metabolites, ${doc.reactions.length} reactions)`);
}

await writeFile(join(outDir, 'index.json'), `${JSON.stringify({ models: index }, null, 2)}\n`, 'utf8');
console.log(`wrote models/index.json  (${index.length} models)`);
