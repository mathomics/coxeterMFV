#!/usr/bin/env node
/**
 * Build a single self-contained HTML file.
 *
 * ES modules and fetch() both refuse to work over file://, so opening
 * index.html by double-clicking it does nothing. This bundles every module into
 * one classic script, inlines the stylesheets and embeds the model catalogue as
 * data, so the result opens straight from the file system with no server, no
 * Node and no install.
 *
 * The bundler is deliberately small: it resolves the import graph, sorts it,
 * and rewrites the handful of import/export forms this codebase actually uses.
 * It is not a general-purpose bundler and will complain loudly rather than
 * guess if it meets a form it does not recognise.
 *
 * Usage: node tools/build-standalone.mjs [salida.html]
 */
import { readFile, writeFile, readdir } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const out = process.argv[2] || join(root, 'coxeter_mfv.html');
const ENTRY = 'src/ui/app.js';
const WORKER_ENTRY = 'src/communities/steadycom-worker.js';
const pkg = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));
const git = (args, fallback = null) => { try { return execFileSync('git', args, { cwd: root, encoding:'utf8' }).trim() || fallback; } catch { return fallback; } };
const buildIdentity = { version: pkg.version, gitCommit: process.env.COXETER_GIT_COMMIT || git(['rev-parse','HEAD']), gitBranch: process.env.COXETER_GIT_BRANCH || git(['rev-parse','--abbrev-ref','HEAD']), buildTime: new Date().toISOString() };

// Bundling

const modules = new Map();   // id (repo-relative) -> { code, deps, exports }

const IMPORT_NAMED = /^import\s*\{([\s\S]*?)\}\s*from\s*['"]([^'"]+)['"];?[ \t]*$/gm;
const IMPORT_BARE = /^import\s*['"]([^'"]+)['"];?[ \t]*$/gm;
const ANY_IMPORT = /^\s*import\s/m;
const EXPORT_DECL = /^export\s+(async\s+)?(function|class|const|let|var)\s+([A-Za-z_$][\w$]*)/gm;
const EXPORT_LIST = /^export\s*\{([\s\S]*?)\}\s*;?[ \t]*$/gm;

async function load(id) {
  if (modules.has(id)) return;
  modules.set(id, null); // placeholder guards against cycles

  const source = await readFile(join(root, id), 'utf8');
  const deps = [];
  const exports = new Set();
  let code = source;

  code = code.replace(IMPORT_NAMED, (_, names, spec) => {
    const dep = resolveSpec(id, spec);
    deps.push(dep);
    const bindings = names
      .split(',')
      .map((part) => part.trim())
      .filter(Boolean)
      .map((part) => {
        const m = /^([A-Za-z_$][\w$]*)(?:\s+as\s+([A-Za-z_$][\w$]*))?$/.exec(part);
        if (!m) throw new Error(`${id}: cannot read import binding "${part}"`);
        return m[2] ? `${m[1]}: ${m[2]}` : m[1];
      })
      .join(', ');
    return `const { ${bindings} } = __mod(${JSON.stringify(dep)});`;
  });

  code = code.replace(IMPORT_BARE, (_, spec) => {
    const dep = resolveSpec(id, spec);
    deps.push(dep);
    return `__mod(${JSON.stringify(dep)});`;
  });

  if (ANY_IMPORT.test(code)) {
    throw new Error(`${id}: an import form this bundler does not handle is left over`);
  }

  for (const match of code.matchAll(EXPORT_DECL)) exports.add(match[3]);
  for (const match of code.matchAll(EXPORT_LIST)) {
    for (const part of match[1].split(',')) {
      const trimmed = part.trim();
      if (!trimmed) continue;
      const m = /^([A-Za-z_$][\w$]*)(?:\s+as\s+([A-Za-z_$][\w$]*))?$/.exec(trimmed);
      if (!m) throw new Error(`${id}: cannot read export "${trimmed}"`);
      exports.add(m[2] ?? m[1]);
    }
  }
  if (/^export\s+default/m.test(code)) {
    throw new Error(`${id}: default exports are not supported by this bundler`);
  }

  code = code.replace(EXPORT_DECL, (_, asyncKeyword, kind, name) => `${asyncKeyword ?? ''}${kind} ${name}`);
  code = code.replace(EXPORT_LIST, '');

  // `import.meta` is a syntax error in a classic script, so one occurrence
  // anywhere stops the whole bundle from parsing and the page comes up blank.
  // Both registries provide a base URL: the page's file URL in the main thread
  // and the Blob URL in its Worker. GLPK receives embedded bytes and a locateFile
  // override, so neither context needs an external WASM/module fetch.
  code = code.replace(/\bimport\.meta\.url\b/g, 'globalThis.__COXETER_MODULE_BASE__');
  if (/\bimport\.meta\b/.test(code)) {
    throw new Error(`${id}: import.meta cannot be bundled into a classic script`);
  }

  modules.set(id, { code, deps: [...new Set(deps)], exports: [...exports] });
  for (const dep of modules.get(id).deps) await load(dep);
}

function resolveSpec(fromId, spec) {
  if (!spec.startsWith('.')) throw new Error(`${fromId}: bare import "${spec}" cannot be bundled`);
  return relative(root, resolve(dirname(join(root, fromId)), spec)).split('\\').join('/');
}

/** Depth-first order so a module is defined before anything that needs it. */
function order(entry) {
  const sorted = [];
  const state = new Map();
  const visit = (id) => {
    if (state.get(id) === 'done') return;
    if (state.get(id) === 'open') return;     // cycle: the registry handles it
    state.set(id, 'open');
    for (const dep of modules.get(id).deps) visit(dep);
    state.set(id, 'done');
    sorted.push(id);
  };
  visit(entry);
  return sorted;
}

await load(ENTRY);
await load(WORKER_ENTRY);
const ordered = order(ENTRY);
const workerOrdered = order(WORKER_ENTRY);

function bundleFor(ids) { return ids.map((id) => {
  const mod = modules.get(id);
  const exported = mod.exports.length
    ? `\n  Object.assign(__exports, { ${mod.exports.join(', ')} });`
    : '';
  return `__define(${JSON.stringify(id)}, function (__exports, __mod) {\n${mod.code}${exported}\n});`;
}).join('\n\n'); }
const bundle = bundleFor(ordered);

/** A separate registry is necessary: Workers cannot read the page's globals. */
function registrySource(source, entry) {
  return `(function () {
  'use strict';
  var __registry = Object.create(null);
  var __cache = Object.create(null);
  function __define(id, factory) { __registry[id] = factory; }
  function __mod(id) {
    if (__cache[id]) return __cache[id];
    var factory = __registry[id];
    if (!factory) throw new Error('Module not found: ' + id);
    var exports = __cache[id] = {};
    factory(exports, __mod);
    return exports;
  }
${source}
  __mod(${JSON.stringify(entry)});
})();`;
}

const [vendorGlue, vendorWasm, vendorProvenance] = await Promise.all([
  readFile(join(root, 'assets/vendor/glpk.js'), 'utf8'),
  readFile(join(root, 'assets/vendor/glpk.wasm')),
  readFile(join(root, 'assets/vendor/PROVENANCE.md'), 'utf8'),
]);
const defaultExport = /export\s*\{\s*([A-Za-z_$][\w$]*)\s+as\s+default\s*\}\s*;?\s*$/;
const vendorExport = defaultExport.exec(vendorGlue);
if (!vendorExport) throw new Error('The vendored GLPK factory export was not recognised.');
const embeddedGlue = vendorGlue.replace(defaultExport,
  `globalThis.__GLPK_FACTORY__ = (options) => ${vendorExport[1]}({ ...options, locateFile: () => 'embedded-glpk.wasm' });`)
  .replace(/\bimport\.meta\.url\b/g, 'globalThis.__COXETER_MODULE_BASE__');
if (/\bimport\.meta\b|\bexport\s*\{/.test(embeddedGlue)) throw new Error('The embedded GLPK factory still has module-only syntax.');
const glpkBootstrap = `/* glpk.js 5.0.0 / GNU GLPK: GPL-3.0; see embedded third-party notices. */
(function () {
${embeddedGlue}
globalThis.__GLPK_WASM__ = Uint8Array.from(atob(${JSON.stringify(vendorWasm.toString('base64'))}), c => c.charCodeAt(0));
})();`;
const workerSource = `globalThis.__COXETER_MODULE_BASE__ = self.location.href;
${glpkBootstrap}
${registrySource(bundleFor(workerOrdered), WORKER_ENTRY)}`;
const thirdPartyNotices = {
  name: 'glpk.js and GNU GLPK', version: '5.0.0', license: 'GPL-3.0',
  sourceURLs: ['https://github.com/jvail/glpk.js', 'https://www.gnu.org/software/glpk/',
    'https://www.npmjs.com/package/glpk.js/v/5.0.0'],
  licenseURL: 'https://www.gnu.org/licenses/gpl-3.0.html',
  provenance: vendorProvenance,
};
const scriptJSON = (value) => JSON.stringify(value).replace(/</g, '\\u003c');

// Data

const dataFiles = ['models/index.json', 'models/scenarios.json'];
const index = JSON.parse(await readFile(join(root, 'models/index.json'), 'utf8'));
for (const entry of index.models) dataFiles.push(`models/${entry.file}`);

/*
 * The published communities need their manifest and their map excerpts embedded
 * too, or selecting one in this build draws nothing. Full member models and
 * community medium/bounds are also included, so fixed-µ SteadyCom uses the same
 * complete biological problem offline while keeping a small curated drawing.
 */
const publishedDir = join(root, 'models', 'published');
const publishedIds = (await readdir(publishedDir, { withFileTypes: true }))
  .filter((e) => e.isDirectory()).map((e) => e.name).sort();
const textFiles = [];
for (const id of publishedIds) {
  dataFiles.push(`models/published/${id}/manifest.json`);
  const manifest = JSON.parse(await readFile(join(publishedDir, id, 'manifest.json'), 'utf8'));
  for (const member of manifest.members ?? []) {
    if (member.preview) dataFiles.push(`models/published/${id}/${member.preview}`);
    if (member.model) dataFiles.push(`models/published/${id}/${member.model}`);
  }
  if (manifest.joint) dataFiles.push(`models/published/${id}/${manifest.joint}`);
  for (const file of [manifest.map, manifest.results]) {
    if (file) dataFiles.push(`models/published/${id}/${file}`);
  }
  for (const file of [manifest.media, manifest.bounds]) {
    if (file) textFiles.push(`models/published/${id}/${file}`);
  }
}

// The scenarios' curated maps (a scenario's `map`), or this build would fall
// back to the automatic layout for every one of them.
const mapNames = await readdir(join(root, 'models', 'maps')).catch(() => []);
for (const name of mapNames.filter((n) => n.endsWith('.json')).sort()) dataFiles.push(`models/maps/${name}`);

const data = {};
for (const file of new Set(dataFiles)) data[file] = JSON.parse(await readFile(join(root, file), 'utf8'));
const text = {};
for (const file of new Set(textFiles)) text[file] = await readFile(join(root, file), 'utf8');

const [appCss, mapCss, indexHtml] = await Promise.all([
  readFile(join(root, 'assets/app.css'), 'utf8'),
  readFile(join(root, 'assets/map.css'), 'utf8'),
  readFile(join(root, 'index.html'), 'utf8'),
]);

// Reuse the real page's markup so the two builds cannot drift apart.
const body = /<body>([\s\S]*?)<\/body>/.exec(indexHtml)[1]
  .replace(/<script[\s\S]*?<\/script>/g, '')
  .trim();
// Keep theme preference initialization before styles in both entry points.
const themeBootstrap = /<script id="theme-init">[\s\S]*?<\/script>/.exec(indexHtml)?.[0];
if (!themeBootstrap) throw new Error('index.html: theme initialization script is missing');

const html = `<!doctype html>
<html lang="en" data-theme="light">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Coxeter Multicell Flux Visualizer — standalone</title>
<meta name="description" content="Metabolic flux visualization with explicit cell and compartment boundaries and multispecies community FBA. A single HTML file that opens without a server.">
<!-- Includes glpk.js / GNU GLPK, GPL-3.0. Source and licence URLs and the
     complete vendored provenance notice are preserved below. App source
     licensing remains as stated by the project; this file includes GPL code. -->
${themeBootstrap}
<style>
${appCss}
${mapCss}
</style>
</head>
<body>
${body}

<script>
window.__COXETER_BUILD__ = ${scriptJSON(buildIdentity)};
/* Modelos y escenarios incrustados: fetch() no funciona sobre file://. */
window.__ESCHER_MCMC_DATA__ = ${scriptJSON(data)};
window.__ESCHER_MCMC_TEXT__ = ${scriptJSON(text)};
globalThis.__COXETER_MODULE_BASE__ = document.baseURI;
globalThis.__COXETER_THIRD_PARTY_NOTICES__ = ${scriptJSON(thirdPartyNotices)};
globalThis.__COXETER_STEADYCOM_WORKER_SOURCE__ = ${scriptJSON(workerSource)};
</script>

<script>
${glpkBootstrap}
/*
 * Registro de módulos mínimo. Cada módulo del proyecto se define abajo tal cual
 * está en src/, con sus import/export reescritos por tools/build-standalone.mjs.
 */
(function () {
  'use strict';
  var __registry = Object.create(null);
  var __cache = Object.create(null);
  function __define(id, factory) { __registry[id] = factory; }
  function __mod(id) {
    if (__cache[id]) return __cache[id];
    var factory = __registry[id];
    if (!factory) throw new Error('Module not found: ' + id);
    var exports = __cache[id] = {};
    factory(exports, __mod);
    return exports;
  }

${bundle}

  __mod(${JSON.stringify(ENTRY)});
})();
</script>
</body>
</html>
`;

await writeFile(out, html, 'utf8');
const kb = Math.round(Buffer.byteLength(html) / 1024);
console.log(`wrote ${relative(root, out) || out}  (${kb} KB, ${ordered.length} app modules, ${workerOrdered.length} Worker modules, ${Object.keys(data).length} JSON files, ${Object.keys(text).length} text files)`);
console.log('Open it directly in a browser — no server needed.');
