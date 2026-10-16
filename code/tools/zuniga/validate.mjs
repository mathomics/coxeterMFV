#!/usr/bin/env node
/**
 * Local validation of the Zuñiga et al. (2020) reproduction: solve every
 * scenario on the authors' original files and check it against the published
 * predictions. Supply the original files privately and never bundle them.
 *
 *   node tools/zuniga/validate.mjs --source-dir path/to/authors --output-dir local-results/zuniga-js
 *
 * Optional --python-results-dir and --matlab-results-dir compare independent
 * recalculations.
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  ZUNIGA_SCENARIOS, loadZunigaStudy, createZunigaScenario, solveZunigaScenario,
} from '../../src/communities/zuniga.js';
import { initGLPK } from '../../src/solver/glpk.js';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));

function optionsFromArgs(args) {
  const options = {};
  for (let i = 0; i < args.length; i++) {
    const flag = args[i];
    if (flag === '--help') return { help: true };
    if (!['--source-dir', '--output-dir', '--python-results-dir', '--matlab-results-dir'].includes(flag)) {
      throw new Error('Unknown option: ' + flag);
    }
    if (!args[i + 1] || args[i + 1].startsWith('--')) throw new Error('Missing value for ' + flag);
    options[flag.slice(2)] = path.resolve(args[++i]);
  }
  if (!options['source-dir'] || !options['output-dir']) throw new Error('--source-dir and --output-dir are required');
  return options;
}

function floatHash(values) {
  const bytes = Buffer.alloc(values.length * 8);
  values.forEach((value, index) => bytes.writeDoubleLE(value, index * 8));
  return createHash('sha256').update(bytes).digest('hex');
}

function sourceSignature(doc) {
  const index = new Map(doc.metabolites.map((met, i) => [met.id, i]));
  const triples = [];
  doc.reactions.forEach((reaction, j) => {
    for (const [mid, value] of Object.entries(reaction.metabolites)) triples.push([index.get(mid), j, value]);
  });
  triples.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  return {
    reactionCount: doc.reactions.length,
    metaboliteCount: doc.metabolites.length,
    matrixNonzeros: triples.length,
    stoichiometrySha256: floatHash(triples.flat()),
    lowerBoundsSha256: floatHash(doc.reactions.map(r => r.lower_bound)),
    upperBoundsSha256: floatHash(doc.reactions.map(r => r.upper_bound)),
    objectiveSha256: floatHash(doc.reactions.map(r => r.objective_coefficient)),
    rhsSha256: floatHash(doc.metabolites.map(() => 0)),
  };
}

function checkNamespace(original, prepared) {
  const reactions = [...prepared.model.reactions.values()];
  const metabolites = [...prepared.model.metabolites.values()];
  assert.equal(reactions.length, original.reactions.length, 'No original reaction may be added or removed');
  assert.equal(metabolites.length, original.metabolites.length, 'No original metabolite may be added or removed');
  assert.equal(prepared.model.genes.length, original.genes.length, 'All source genes must remain');
  const metIds = new Map(metabolites.map((met, i) => [original.metabolites[i].id, met.id]));
  metabolites.forEach((met, i) => assert.equal(met.notes.source_metabolite_id, original.metabolites[i].id));
  let knockoutChanges = 0;
  reactions.forEach((reaction, j) => {
    const source = original.reactions[j];
    assert.equal(reaction.notes.source_reaction_id, source.id);
    assert.deepEqual(reaction.metabolites, Object.fromEntries(
      Object.entries(source.metabolites).map(([mid, value]) => [metIds.get(mid), value]),
    ), 'Every original stoichiometric coefficient must remain after namespace conversion');
    assert.equal(reaction.objectiveCoefficient, source.objective_coefficient);
    if (prepared.scenario.mutant && reaction.id === 'Ec@PPNDH') {
      assert.equal(reaction.lowerBound, 0);
      assert.equal(reaction.upperBound, 0);
      knockoutChanges++;
    } else {
      assert.ok(reaction.lowerBound === source.lower_bound, 'Original lower bound changed');
      assert.ok(reaction.upperBound === source.upper_bound, 'Original upper bound changed');
    }
  });
  assert.equal(knockoutChanges, prepared.scenario.mutant ? 1 : 0);
  return {
    originalReactionOrderPreserved: true,
    originalMetaboliteOrderPreserved: true,
    stoichiometryAndOriginalBoundsExact: true,
    sourceObjectiveExact: true,
    sourceGeneCount: original.genes.length,
    knockoutChangedReactionCount: knockoutChanges,
  };
}

function numericalResidual(prepared, result) {
  const p = prepared.problem;
  const residual = Float64Array.from(p.b, value => -value);
  let boundViolation = 0;
  p.rxnIds.forEach((rid, j) => {
    const flux = result.fluxes.get(rid);
    assert.ok(Number.isFinite(flux), 'Every reaction must have a finite solved flux');
    const column = p.A.column(j);
    for (let k = 0; k < column.length; k += 2) residual[column[k]] += column[k + 1] * flux;
    boundViolation = Math.max(boundViolation, p.lb[j] - flux, flux - p.ub[j]);
  });
  return {
    maxMassBalanceResidual: Math.max(0, ...Array.from(residual, Math.abs)),
    maxBoundViolation: boundViolation,
  };
}

function close(actual, expected, tolerance, description) {
  assert.ok(Math.abs(actual - expected) <= tolerance,
    description + ': actual=' + actual + ', expected=' + expected + ', tolerance=' + tolerance);
}

/**
 * Run the validation with options keyed like the flags ('source-dir',
 * 'output-dir', optional 'python-results-dir', 'matlab-results-dir'). Writes
 * validation.json and one JSON per scenario to the output directory, returns
 * the report and throws on any mismatch.
 */
export async function validateZunigaStudy(options) {
  const manifest = JSON.parse(await readFile(path.join(ROOT, 'models/published/zuniga-2020-phea-rescue/manifest.json'), 'utf8'));
  const sourceNames = new Set(manifest.sources.map(source => source.file));
  const fetcher = async url => {
    const filename = path.posix.basename(new URL(url).pathname);
    assert.ok(sourceNames.has(filename), 'Only pinned manifest source files may be read');
    const bytes = await readFile(path.join(options['source-dir'], filename));
    return new Response(bytes, { status: 200 });
  };
  globalThis.__GLPK_FACTORY__ = (await import('../../assets/vendor/glpk.js')).default;
  globalThis.__GLPK_WASM__ = new Uint8Array(await readFile(path.join(ROOT, 'assets/vendor/glpk.wasm')));
  assert.equal(await initGLPK(), true, 'The bundled WebAssembly GLPK backend must initialize');
  const study = await loadZunigaStudy(manifest, { fetcher });
  const scenarios = [];
  for (const scenario of ZUNIGA_SCENARIOS) {
    const prepared = createZunigaScenario(study, scenario.id);
    const source = scenario.coculture ? study.sourceModels.joint : study.sourceModels.mono;
    const namespace = checkNamespace(source, prepared);
    const result = solveZunigaScenario(prepared);
    assert.equal(result.status, 'optimal', 'The original stationary FBA must be optimal');
    const growth = Object.fromEntries(prepared.community.growthRates(result.fluxes));
    const residual = numericalResidual(prepared, result);
    close(residual.maxMassBalanceResidual, 0, 1e-7, 'Mass balance residual');
    close(residual.maxBoundViolation, 0, 1e-7, 'Bound violation');
    const record = {
      id: scenario.id, source: prepared.source, method: result.zuniga.method,
      status: result.status, objectiveValue: result.objectiveValue, growth,
      knockouts: prepared.knockouts, fluxes: Object.fromEntries(result.fluxes),
      originalFluxes: result.zuniga.originalFluxes,
      sourceSignature: sourceSignature(source),
      validation: { ...namespace, ...residual },
      crossFeeding: prepared.community.crossFeeding(result.fluxes),
    };
    console.log(scenario.id + ': objective=' + result.objectiveValue + ' growth=' + JSON.stringify(growth));
    if (options['python-results-dir']) {
      const independent = JSON.parse(await readFile(path.join(options['python-results-dir'], scenario.id + '.json'), 'utf8'));
      assert.deepEqual(record.sourceSignature, independent.source, 'JS and Python source arrays differ');
      close(record.objectiveValue, independent.objective, 1e-8, 'Python/JS objective agreement');
      const ecId = scenario.coculture ? 'BIOMASS_Ec_iJO1366_core_53p95M_D' : 'BIOMASS_Ec_iJO1366_core_53p95M';
      close(growth.Ec, independent.growthObjectiveReactions[ecId], 1e-8, 'Python/JS E. coli agreement');
      if (scenario.coculture) close(growth.Se, independent.growthObjectiveReactions.BOF_A, 1e-8, 'Python/JS S. elongatus agreement');
      assert.equal(record.validation.sourceGeneCount, independent.geneCount);
      record.validation.pythonIndependentMatch = true;
    }
    if (options['matlab-results-dir']) {
      const independent = JSON.parse(await readFile(path.join(options['matlab-results-dir'], scenario.id + '.json'), 'utf8'));
      close(record.objectiveValue, independent.objective, 1e-8, 'MATLAB/JS objective agreement');
      for (const reaction of (Array.isArray(independent.growth) ? independent.growth : [independent.growth])) {
        close(record.originalFluxes[reaction.reaction], reaction.flux, 1e-8, 'MATLAB/JS biomass agreement');
      }
      record.validation.matlabIndependentMatch = true;
    }
    scenarios.push(record);
  }
  const mono = scenarios.find(s => s.id === 'mutant-monoculture');
  const co = scenarios.find(s => s.id === 'mutant-coculture');
  const ref = manifest.reproduction.referenceValues;
  close(mono.growth.Ec, ref.mutantMonocultureEc, 1e-8, 'Published mutant monoculture');
  close(co.growth.Ec, ref.mutantCocultureEc, 1e-8, 'Published E. coli rescue');
  close(co.growth.Se, ref.mutantCocultureSe, 1e-8, 'Published S. elongatus growth');
  close(co.fluxes['Ec@PHEtex'], ref.PHEtex_D, 1e-8, 'Published phenylalanine uptake');
  close(co.fluxes['Se@PHELtex'], ref.PHELtex_A, 1e-8, 'Published phenylalanine secretion');
  assert.equal(co.fluxes['Ec@PPNDH'], 0);
  assert.equal(co.fluxes.EX_phe__L_s, 0);
  const link = co.crossFeeding.links.find(link => link.base === 'phe__L' && link.from === 'Se' && link.to === 'Ec');
  assert.ok(link, 'Rescue cross-feeding must be assigned by stoichiometric net direction');
  close(link.flux, Math.abs(ref.PHEtex_D), 1e-8, 'Phenylalanine transfer magnitude');
  const report = {
    schemaVersion: 1, caseId: manifest.id, generatedBy: 'Coxeter exact source-matrix FBA',
    sourceHashes: study.sourceHashes, publishedPredictionsMatched: true, tolerance: 1e-8,
    scenarios,
  };
  await mkdir(options['output-dir'], { recursive: true });
  await writeFile(path.join(options['output-dir'], 'validation.json'), JSON.stringify(report, null, 2) + '\n');
  for (const scenario of scenarios) {
    await writeFile(path.join(options['output-dir'], scenario.id + '.json'), JSON.stringify(scenario, null, 2) + '\n');
  }
  console.log('Published predictions matched; output=' + options['output-dir']);
  return report;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const options = optionsFromArgs(process.argv.slice(2));
  if (options.help) {
    console.log('node tools/zuniga/validate.mjs --source-dir AUTHORS_CHECKOUT --output-dir LOCAL_RESULTS [--python-results-dir LOCAL_RESULTS] [--matlab-results-dir LOCAL_RESULTS]');
  } else {
    await validateZunigaStudy(options);
  }
}
