/** Dedicated, disposable Worker for one SteadyCom range, fixed-µ or parsimonious request. */
import { restoreSteadyComSpec } from './steadycom-runtime.js';
import { steadyCom, maxTotalBiomassAt, parsimoniousAt } from '../core/steadycom.js';
import { initGLPK } from '../solver/glpk.js';

self.onmessage = async ({ data }) => {
  const { id, spec: snapshot, request } = data ?? {};
  try {
    if (!['range', 'fixed', 'parsimonious'].includes(request?.type)) throw new Error('Unknown SteadyCom task.');
    if (!await initGLPK()) throw new Error('SteadyCom needs the GLPK backend, which did not load in the Worker.');
    const spec = restoreSteadyComSpec(snapshot);
    const options = { ...request.options, onProgress: (progress) => {
      self.postMessage({ id, type: 'progress', progress });
    } };
    const result = request.type === 'range' ? steadyCom(spec, options)
      : request.type === 'parsimonious' ? parsimoniousAt(spec, request.mu, options)
      : maxTotalBiomassAt({ ...spec, biomassCap: null }, request.mu, { ...options, biomassCap: null });
    self.postMessage({ id, type: 'result', result });
  } catch (error) {
    self.postMessage({ id, type: 'error', error: { name: error.name || 'Error', message: error.message || String(error) } });
  }
};
