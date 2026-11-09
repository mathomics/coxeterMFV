import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import factory from '../assets/vendor/glpk.js';
import { initGLPK } from '../src/solver/glpk.js';
import { Model } from '../src/core/model.js';
import { jointSteadyComSpec } from '../src/communities/steadycom-runtime.js';
import { maxTotalBiomassAt } from '../src/core/steadycom.js';

test('tiny populations in the full nine-species community retain their requested specific growth', async () => {
  globalThis.__GLPK_FACTORY__ = factory;
  globalThis.__GLPK_WASM__ = await readFile(new URL('../assets/vendor/glpk.wasm', import.meta.url));
  assert.equal(await initGLPK(), true);
  const directory = new URL('../models/published/steadycom-gut-nine/', import.meta.url);
  const manifest = JSON.parse(await readFile(new URL('manifest.json', directory), 'utf8'));
  const members = await Promise.all(manifest.members.map(async member => ({ id: member.id,
    model: new Model(JSON.parse(await readFile(new URL(member.model, directory), 'utf8'))) })));
  const bounds = await readFile(new URL('bounds.csv', directory), 'utf8');
  for (const { mu, fiber } of [{ mu: 0.03984680175781251 }, { mu: 0.04 },
    { mu: 0.036019897460937506, fiber: { bacteroides: 10, faecalibacterium: 5 } }]) {
  const spec = jointSteadyComSpec(members, bounds, { fiber });
  const result = maxTotalBiomassAt(spec, mu, { timeLimitSeconds: 30 });
  assert.equal(result.status, 'optimal');
  assert.equal(result.feasible, true);
  assert.ok(Math.abs(result.total - 1) < 1e-8);
  assert.ok([...result.abundances.values()].some(x => x > 1e-9 && x < 1e-3));
  for (const member of spec.members) {
    const x = result.abundances.get(member.id);
    const biomass = result.fluxes.get(member.biomassReaction);
    if (x > 1e-9) assert.ok(Math.abs(biomass / x - mu) < 1e-7, `${member.id}: biomass/X must equal the requested μ`);
    else assert.ok(Math.abs(biomass) < 1e-8);
  }
  const balances = new Map();
  for (const [id, reaction] of spec.reactions) {
    const value = result.fluxes.get(id);
    const x = reaction.species === 'comm' ? 1 : result.abundances.get(reaction.species);
    const scale = Math.max(1, Math.abs(reaction.lowerBound), Math.abs(reaction.upperBound));
    assert.ok(value >= reaction.lowerBound * x - 1e-5 * scale, `${id}: lower bound`);
    assert.ok(value <= reaction.upperBound * x + 1e-5 * scale, `${id}: upper bound`);
    for (const [mid, coefficient] of Object.entries(reaction.metabolites)) balances.set(mid, (balances.get(mid) ?? 0) + coefficient * value);
  }
  for (const [mid, residual] of balances) assert.ok(Math.abs(residual) < 1e-5, `${mid}: mass balance ${residual}`);
  }
});
