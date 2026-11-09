import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, mkdtemp, rm } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { tmpdir } from 'node:os';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const run = promisify(execFile);

test('standalone packages complete SteadyCom inputs and an independent GLPK Worker', async () => {
  const tempRoot = resolve(tmpdir());
  const directory = await mkdtemp(join(tempRoot, 'coxeter-standalone-test-'));
  try {
    const file = join(directory, 'offline.html');
    await run(process.execPath, [join(root, 'tools/build-standalone.mjs'), file], { cwd: root });
    const html = await readFile(file, 'utf8');
    const scripts = [...html.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)].map((m) => m[1]);
    const globals = scripts.find((script) => script.includes('window.__ESCHER_MCMC_DATA__ ='));
    assert.ok(globals);
    const context = { document: { baseURI: 'file:///offline.html' } };
    context.window = context;
    context.globalThis = context;
    vm.runInNewContext(globals, context, { timeout: 10000 });
    const data = context.__ESCHER_MCMC_DATA__;
    const text = context.__ESCHER_MCMC_TEXT__;
    for (const id of ['steadycom-ecoli-auxotrophs', 'steadycom-gut-nine']) {
      const path = `models/published/${id}/`;
      const manifest = data[`${path}manifest.json`];
      assert.ok(manifest.members.length >= 4);
      for (const member of manifest.members) {
        assert.equal(data[`${path}${member.model}`].reactions.length, member.reactions);
        assert.ok(data[`${path}${member.model}`].reactions.length > data[`${path}${member.preview}`].reactions.length);
        assert.ok(data[`${path}${member.preview}`].reactions.length > 0);
      }
      assert.equal(text[`${path}${manifest.media}`], await readFile(join(root, path, manifest.media), 'utf8'));
      assert.equal(text[`${path}${manifest.bounds}`], await readFile(join(root, path, manifest.bounds), 'utf8'));
    }
    for (const entry of data['models/index.json'].models) assert.ok(data[`models/${entry.file}`]);
    // A scenario's curated map ships whenever its file is there.
    for (const scenario of data['models/scenarios.json'].scenarios.filter((s) => s.map)) {
      const present = await readFile(join(root, 'models', scenario.map)).then(() => true, () => false);
      assert.equal(Boolean(data[`models/${scenario.map}`]), present, scenario.map);
    }
    const studyPath = 'models/published/zuniga-2020-phea-rescue/';
    const study = data[`${studyPath}manifest.json`];
    assert.equal(study.runtime, 'zuniga-mat');
    assert.equal(data[`${studyPath}${study.results}`].scenarios.length, 3);
    assert.equal(data[`${studyPath}${study.map}`].reactions.length, 25);
    for (const member of study.members) assert.equal(member.model, null,
      'Unlicensed full third-party models must be downloaded privately, not bundled.');
    assert.ok(scripts.some(script => script.includes('__define("src/import/mat5.js"')));
    const workerSource = context.__COXETER_STEADYCOM_WORKER_SOURCE__;
    assert.equal(typeof workerSource, 'string');
    assert.doesNotThrow(() => new vm.Script(workerSource));
    assert.ok(workerSource.includes('self.onmessage'));
    assert.ok(workerSource.includes('globalThis.__GLPK_FACTORY__'));
    assert.ok(workerSource.includes('globalThis.__GLPK_WASM__'));
    assert.ok(workerSource.includes('__define("src/communities/steadycom-worker.js"'));
    assert.ok(scripts.some((script) => script.includes('globalThis.__GLPK_FACTORY__ =')));
    const notice = context.__COXETER_THIRD_PARTY_NOTICES__;
    assert.equal(notice.license, 'GPL-3.0');
    assert.equal(notice.provenance, await readFile(join(root, 'assets/vendor/PROVENANCE.md'), 'utf8'));
    assert.ok(notice.sourceURLs.includes('https://github.com/jvail/glpk.js'));
  } finally {
    const target = resolve(directory);
    if (!target.startsWith(`${tempRoot}${sep}`)) throw new Error('Refusing to remove a temporary build outside its parent.');
    await rm(target, { recursive: true, force: true });
  }
});
