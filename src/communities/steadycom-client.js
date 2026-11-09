/** Run a complete SteadyCom request away from the browser's UI thread. */
import { serializeSteadyComSpec, restoreSteadyComSpec } from './steadycom-runtime.js';
import { steadyCom, maxTotalBiomassAt, parsimoniousAt } from '../core/steadycom.js';
import { initGLPK } from '../solver/glpk.js';

let nextTaskId = 0;

function abortError(message) {
  const error = new Error(message);
  error.name = 'AbortError';
  return error;
}

function validateRequest(request) {
  if (!request || !['range', 'fixed', 'parsimonious'].includes(request.type)) {
    throw new Error('Choose a SteadyCom range search or a fixed growth-rate solve.');
  }
  if (request.type !== 'range' && (!Number.isFinite(request.mu) || request.mu < 0)) {
    throw new Error('The fixed growth rate must be a finite number of zero or more.');
  }
  if (request.options !== undefined && (!request.options || typeof request.options !== 'object'
      || Array.isArray(request.options))) throw new Error('SteadyCom options must be an object.');
}

/**
 * Run one SteadyCom request: the growth-rate search ('range'), the maximum
 * total biomass at a fixed mu ('fixed') or the parsimonious solution at mu.
 * Browsers solve in a Web Worker, Node solves in-process. The promise rejects
 * with an AbortError on cancel or after `timeoutMs` (default 300000 ms).
 *
 * @param {object} spec SteadyCom spec; member models and Maps are copied.
 * @param {{type:'range'|'fixed'|'parsimonious',mu?:number,options?:object}} request
 * @param {{onProgress?:function,signal?:AbortSignal,timeoutMs?:number}} controls
 * @returns {Promise<object>} Core result, including native Maps.
 */
export function runSteadyComTask(spec, request, {
  onProgress = null, signal = null, timeoutMs = 300000,
} = {}) {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) { reject(abortError('SteadyCom was cancelled.')); return; }
    let snapshot;
    let task;
    try {
      validateRequest(request);
      if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) throw new Error('The task timeout must be a finite positive number.');
      if (onProgress !== null && typeof onProgress !== 'function') throw new Error('The progress callback must be a function.');
      snapshot = serializeSteadyComSpec(spec);
      // Progress functions belong to this client, never to a structured clone.
      const { onProgress: ignored, ...options } = request.options ?? {};
      task = { type: request.type, mu: request.mu, options };
    } catch (error) { reject(error); return; }

    const id = ++nextTaskId;
    let worker = null;
    let objectURL = null;
    let settled = false;
    let timer;
    const finish = (error, result) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      signal?.removeEventListener('abort', cancel);
      if (worker) {
        worker.onmessage = null;
        worker.onerror = null;
        worker.onmessageerror = null;
        worker.terminate();
      }
      if (objectURL) URL.revokeObjectURL(objectURL);
      if (error) reject(error); else resolve(result);
    };
    const cancel = () => finish(abortError('SteadyCom was cancelled.'));
    const progress = (value) => {
      if (settled) return;
      try { onProgress?.(value); }
      catch (error) { finish(error); }
    };
    signal?.addEventListener('abort', cancel, { once: true });
    timer = setTimeout(() => finish(abortError(`SteadyCom exceeded its ${timeoutMs} ms task timeout.`)), timeoutMs);

    // Node's direct fallback is useful for headless validation. Browser callers
    // require a Worker: a synchronous genome-scale solve must never freeze the UI.
    const browser = typeof window !== 'undefined' || typeof document !== 'undefined';
    if (browser) {
      try {
        if (typeof Worker !== 'function') throw new Error('SteadyCom requires Web Workers in this browser.');
        const embedded = globalThis.__COXETER_STEADYCOM_WORKER_SOURCE__;
        let url = new URL('./steadycom-worker.js', import.meta.url);
        let workerType = 'module';
        if (typeof embedded === 'string' && embedded.trim()) {
          objectURL = URL.createObjectURL(new Blob([embedded], { type: 'text/javascript' }));
          url = objectURL;
          // The bundled registry has no static imports. A classic Blob Worker
          // also runs from file://, where Chrome rejects module Blob Workers.
          workerType = 'classic';
        }
        worker = new Worker(url, { type: workerType, name: 'Coxeter SteadyCom' });
        worker.onmessage = ({ data }) => {
          if (settled || data?.id !== id) return;
          if (data.type === 'progress') { progress(data.progress); return; }
          if (data.type === 'result') { finish(null, data.result); return; }
          if (data.type === 'error') {
            const error = new Error(data.error?.message || 'The SteadyCom worker failed.');
            error.name = data.error?.name || 'Error';
            finish(error);
          }
        };
        worker.onerror = (event) => finish(new Error(event.message || 'The SteadyCom worker could not load.'));
        worker.onmessageerror = () => finish(new Error('The SteadyCom worker returned an unreadable result.'));
        worker.postMessage({ id, spec: snapshot, request: task });
      } catch (error) { finish(error); }
      return;
    }

    (async () => {
      if (!await initGLPK()) throw new Error('SteadyCom needs the GLPK backend, which did not load.');
      if (settled) return;
      const restored = restoreSteadyComSpec(snapshot);
      const options = { ...task.options, onProgress: (value) => {
        if (signal?.aborted || settled) throw abortError('SteadyCom was cancelled.');
        progress(value);
        if (settled) throw abortError('SteadyCom was cancelled.');
      } };
      const result = task.type === 'range' ? steadyCom(restored, options)
        : task.type === 'parsimonious' ? parsimoniousAt(restored, task.mu, options)
        : maxTotalBiomassAt({ ...restored, biomassCap: null }, task.mu, { ...options, biomassCap: null });
      if (signal?.aborted) throw abortError('SteadyCom was cancelled.');
      finish(null, result);
    })().catch((error) => finish(error));
  });
}
