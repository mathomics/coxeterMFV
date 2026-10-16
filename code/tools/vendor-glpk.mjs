#!/usr/bin/env node
/**
 * Copy glpk.js into assets/vendor/ for the browser, with one documented patch.
 *
 * The direct (non-Worker) build is the one we want: its `solve()` is
 * synchronous, so adopting it does not force the whole flux pipeline to become
 * async. Its Emscripten factory accepts module overrides, including
 * `wasmBinary`, which a browser needs because the glue would otherwise try to
 * fetch the .wasm synchronously and fail.
 *
 * The published ESM export, however, is a zero-argument wrapper:
 *
 *     var t = () => r(); export { t as default };
 *
 * so every override is silently dropped. `r` itself does accept them
 * (`async function (r = null) { const t = typeof r === 'object' ? r : { wasmBinary: r }; ... }`),
 * so forwarding the arguments is enough. That is the single change made here,
 * and it is applied mechanically so the vendored copy can always be rebuilt.
 *
 * Usage: node tools/vendor-glpk.mjs
 */
import { readFile, writeFile, mkdir, copyFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const src = join(root, 'node_modules', 'glpk.js');
const dest = join(root, 'assets', 'vendor');

const PATCH_FROM = 'var t=()=>r();export{t as default}';
const PATCH_TO = 'var t=(...a)=>r(...a);export{t as default}';

const pkg = JSON.parse(await readFile(join(src, 'package.json'), 'utf8'));
await mkdir(dest, { recursive: true });

const glue = await readFile(join(src, 'dist', 'glpk.js'), 'utf8');
if (!glue.includes(PATCH_FROM)) {
  console.error('The expected export wrapper was not found in glpk.js.');
  console.error('glpk.js has changed; re-check the patch before vendoring.');
  process.exit(1);
}
const patched = glue.replace(PATCH_FROM, PATCH_TO);
await writeFile(join(dest, 'glpk.js'), patched, 'utf8');
await copyFile(join(src, 'dist', 'glpk.wasm'), join(dest, 'glpk.wasm'));

const sha = (buf) => createHash('sha256').update(buf).digest('hex').slice(0, 16);
const wasm = await readFile(join(dest, 'glpk.wasm'));

await writeFile(join(dest, 'PROVENANCE.md'), `# assets/vendor

Third-party files, vendored so the browser build loads them without a bundler.
Regenerate with \`npm run vendor:glpk\`; do not edit them by hand.

## glpk.js ${pkg.version}

| | |
|---|---|
| Source | npm \`glpk.js@${pkg.version}\` |
| Upstream | ${pkg.repository?.url?.replace(/^git\+/, '') ?? 'https://github.com/jvail/glpk.js'} |
| Licence | **${pkg.license}** (GLPK itself is GPL; see \`node_modules/glpk.js/LICENSE\`) |
| \`glpk.js\` | Emscripten glue, direct build — sha256 ${sha(patched)}… (patched, see below) |
| \`glpk.wasm\` | GLPK compiled to WebAssembly — sha256 ${sha(wasm)}… (verbatim) |

### The one patch applied

The published ESM export drops the factory's arguments:

\`\`\`js
${PATCH_FROM}
\`\`\`

which makes it impossible to hand the module a pre-fetched \`wasmBinary\`. A
browser needs exactly that, because the glue otherwise attempts a synchronous
fetch of the \`.wasm\` and aborts. The inner factory \`r\` already accepts the
overrides, so the fix is to forward them:

\`\`\`js
${PATCH_TO}
\`\`\`

Nothing else is changed, and \`glpk.wasm\` is copied byte for byte.

### Licensing note

This project's own code is MIT. GLPK is GPL-3.0. It is loaded as an optional
backend: \`src/solver/simplex.js\` remains a complete, dependency-free fallback,
and the app runs without these files. Anyone redistributing a build that bundles
them should treat that build as GPL.
`, 'utf8');

console.log(`vendored glpk.js ${pkg.version} (${pkg.license}) to assets/vendor/`);
console.log(`  glpk.js   ${(patched.length / 1024).toFixed(0)} KB  (export wrapper patched)`);
console.log(`  glpk.wasm ${(wasm.length / 1024).toFixed(0)} KB  (verbatim)`);
