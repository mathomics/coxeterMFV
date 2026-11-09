/**
 * Coxeter-MFV without a browser, for the manuscript analyses.
 *
 * Uses the application's own Store, solver and renderer from src/, the same
 * modules the single HTML file bundles, so a number or figure produced here is
 * what the application produces. Only the parts of app.js that read the screen
 * are replaced: the render state below mirrors mapRenderState() for an
 * exported figure, and the SVG gets map.css inlined as the Export SVG button
 * does.
 */
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const ROOT = fileURLToPath(new URL('../../', import.meta.url));

/** Load the vendored glpk.js 5.0.0, the build the HTML application ships. */
export async function startGLPK() {
  const vendor = path.join(ROOT, 'assets', 'vendor');
  const mod = await import(pathToFileURL(path.join(vendor, 'glpk.js')));
  globalThis.__GLPK_FACTORY__ = mod.default ?? mod;
  globalThis.__GLPK_WASM__ = await readFile(path.join(vendor, 'glpk.wasm'));
  const { initGLPK, glpkAvailable } = await import(pathToFileURL(path.join(ROOT, 'src', 'solver', 'glpk.js')));
  await initGLPK();
  if (!glpkAvailable()) throw new Error('GLPK did not load');
}

/**
 * A Store over this checkout. The store fetches its catalogue and bundled
 * models by path, which Node's fetch cannot do, so local paths are read from
 * disk and anything else goes to the network as before.
 */
export async function createStore({ init = false } = {}) {
  // Node 18 has Web Crypto only as a module export, and the Zuñiga loader
  // hashes the downloaded MAT files with crypto.subtle as a browser does.
  if (!globalThis.crypto) globalThis.crypto = (await import('node:crypto')).webcrypto;
  if (!globalThis.__coxeterLocalFetch) {
    const networkFetch = globalThis.fetch;
    globalThis.fetch = async (url, options) => {
      const target = String(url);
      if (path.isAbsolute(target) || target.startsWith('file:')) {
        const file = target.startsWith('file:') ? fileURLToPath(target) : target;
        try {
          return new Response(await readFile(file));
        } catch {
          return new Response(null, { status: 404 });
        }
      }
      return networkFetch(url, options);
    };
    globalThis.__coxeterLocalFetch = true;
  }
  const { Store } = await import(pathToFileURL(path.join(ROOT, 'src', 'ui', 'store.js')));
  const store = new Store(ROOT.replace(/\/$/, ''));
  if (init) await store.init();
  return store;
}

/** The render state of an exported figure, as app.js builds it for the map on screen. */
export function exportState(store, overrides = {}) {
  const changes = store.constraintChanges?.() ?? [];
  return {
    fluxes: store.displayFluxes(),
    solvedFluxes: store.solution?.status === 'optimal' ? store.solution.fluxes : new Map(),
    reactionWidthMode: store.isZunigaStudy || store.view.widthByFlux ? 'flux' : 'constant',
    offStyle: store.view.dashNoFlux !== false && store.solution?.status === 'optimal',
    lineHops: store.view.lineHops !== false,
    disabledReactionIds: new Set([
      ...(store.isZunigaStudy ? store.zunigaPrepared?.knockouts ?? [] : []),
      ...(store.publishedKnockoutIds?.() ?? []),
      ...changes.filter((change) => change.knockedOut).map((change) => change.id),
    ]),
    modifiedReactionIds: new Set(changes.filter((change) => !change.knockedOut).map((change) => change.id)),
    inactiveSpeciesIds: new Set([
      ...(store.isZunigaStudy && store.zunigaScenarioId !== 'mutant-coculture' ? ['Se'] : []),
      ...(store.disabledDrawnSpeciesIds?.() ?? []),
    ]),
    inactiveSpeciesLabel: store.isZunigaStudy ? 'inactive context' : 'switched off',
    emphasizedReactionIds: new Set(store.isZunigaStudy ? ['Se@PHELtex', 'Ec@PHEtex', 'Ec@PHEt2rpp'] : []),
    scale: store.scale,
    dark: false,
    reactionColorMode: store.view.reactionColorMode,
    labels: store.view.labels,
    metaboliteLabels: store.mapLabels('metabolite'),
    reactionLabels: store.mapLabels('reaction'),
    showCurrency: store.view.showCurrency,
    currencyLabels: store.view.currencyLabels,
    showCrossFeeding: store.crossFeedingHighlighted(),
    crossFeedingWidth: store.view.crossFeedingWidth,
    crossFeeding: store.crossFeeding,
    crossFeedingBases: store.crossFeedingHighlighted() ? store.crossFeedingVisibleBases() : null,
    growth: store.growth,
    forExport: true,
    ...overrides,
  };
}

/** The map as a standalone SVG file, styled as the Export SVG button styles it. */
export async function exportSVG(store, overrides = {}) {
  const { renderMap } = await import(pathToFileURL(path.join(ROOT, 'src', 'render', 'mapview.js')));
  const css = await readFile(path.join(ROOT, 'assets', 'map.css'), 'utf8');
  const svg = renderMap(store.layout, exportState(store, overrides));
  return svg.replace(/(<svg[^>]*>)/, `$1<style>${css}</style>`);
}
