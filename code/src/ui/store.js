/**
 * Application state.
 *
 * Two kinds of change, with very different costs:
 *   structural  species set, abundances, objective  ->  rebuild (+ relayout)
 *   numeric     medium limits, reaction bounds      ->  re-solve only
 *
 * Keeping them apart is what makes dragging a bound slider feel live: only the
 * LP is re-run, the map geometry is untouched.
 *
 * Three layers meet here and are kept apart:
 *
 *   model       CommunityModel — everything the biology makes available
 *   map         EditableMap    — what is currently drawn
 *   simulation  FluxProblem + solution — the LP and its answer
 *
 * The simulation reads the model, never the map, so what is on screen cannot
 * change a flux. The map reads the model when it is built and then owns its own
 * geometry.
 *
 * Medium values are stored as positive magnitudes (mmol/gDW/h available to the
 * community) and converted to the negative lower bound an exchange reaction
 * wants at the point of use.
 */

import { Model } from '../core/model.js';
import { labelSourceName, objectLabel, objectLabelSources } from '../core/label-sources.js';
import { OPERATION_DOMAIN, assertOperationDomain, operationDomain } from './operation-boundary.js';
import { toSBML } from '../core/sbml-write.js';
import { compareReplayedSolution, createReproducibilityBundle, methodsProvenanceReport, softwareIdentity, verifyReproducibilityBundle } from '../core/reproducibility.js';
import { prepareImportedModels } from '../core/model-import.js';
import {
  compareReactionScores, deserializeOmicsDataset, divergingOmicsColors, parseOmicsCSV,
  reactionOmicsScores, sequentialOmicsColors, serializeOmicsDataset,
} from '../omics/overlay.js';
import { communityOmicsMappingReport, parseMetatranscriptomics } from '../omics/community-omics.js';
import { runGIMME } from '../omics/gimme.js';
import { addMetaboliteToModel, addReactionToModel } from '../core/model-edit.js';
import { cellCompartmentName } from '../core/compartments.js';
import { COMPARTMENT_SHAPES } from '../layout/geometry.js';
import { localMetaboliteBase } from '../layout/place-reaction.js';
import { CommunityModel, OBJECTIVES, communityExchange, namespaced, splitId } from '../core/community.js';
import { FluxProblem } from '../solver/fba.js';
import { runSteadyComTask } from '../communities/steadycom-client.js';
import {
  ZUNIGA_ID, ZUNIGA_SCENARIOS, loadZunigaStudy, createZunigaScenario, solveZunigaScenario,
} from '../communities/zuniga.js';
import {
  parseMediaCSV, steadyComSpec, jointSteadyComSpec, jointCommunityProjection, communityBalances, toMergedFluxes,
} from '../communities/steadycom-runtime.js';
import { initGLPK, glpkAvailable } from '../solver/glpk.js';
import { EditableMap } from '../map/editable-map.js';
import { MapHistory } from '../map/history.js';
import {
  createMapDocument, parseMapDocument, validateMapContext, mapSpeciesIds, renameMapSpecies,
} from '../map/map-document.js';
import { reactionsInCompartment } from '../map/compartment-reactions.js';
import {
  pruneMapToModel, reconcileCompartments, describeReconciliation,
} from '../map/reconcile-map.js';
import { emptySelection, parseCompartmentKey, selectionSize } from '../map/multi-selection.js';
import { normalizeHexColor } from '../core/colors.js';
import { exportFileName } from './file-names.js';
import { TEXT_SIZE, DEFAULT_TEXT_FONT, clampTextSize, normalizeTextFont } from '../core/text-style.js';
import { FluxScale, formatFlux, normalizeFluxColorStops } from '../render/theme.js';
import {
  buildReactionColoring, categoryColor, normalizeReactionColorMode, normalizePathwayRibbonMode,
} from '../render/reaction-colors.js';
import { buildCrossFeedingIndex, exchangedMetabolites, filterCrossFeedingIndex } from '../analysis/cross-feeding-explorer.js';
import { isDrawnCrossFeedingBase } from '../render/crossfeed-routes.js';
import {
  YEAST9_MODEL_KEY, YEAST9_SCENARIO_ID, YEAST9_SOURCE,
  YEAST9_PREVIEW_COMPARTMENTS, yeast9PreviewSpecs,
} from '../core/yeast9.js';
import {
  DEFAULT_LAYOUT_PRESET, layoutEngineOptions, layoutPreset, normalizeLayoutSettings,
} from '../layout/layout-settings.js';
import {
  buildCoreMetabolismOverview, coreOverviewSupport,
} from '../map/core-metabolism-overview.js';

// Uploads with more reactions than this, over all species, are not solved on load.
const AUTO_SOLVE_REACTION_LIMIT = 1500;

// Log entries kept. The oldest go first.
const LOG_LIMIT = 200;

/*
 * When an uploaded model is drawn in full without being asked.
 *
 * The automatic layout resolves overlaps inside each compartment, and that cost
 * climbs steeply with how crowded one cell gets. Measured in Chromium on one
 * cell: 95 reactions / 72 metabolites take 0.3 s, 300 / 453 take 1.4 s, 500 /
 * 667 take 3.2 s, and iAF1260 (2383 / 1668) takes over a minute and is
 * unreadable at the end of it. Metabolite count dominates, so both are capped.
 * Below these limits the draw costs about what the bundled scenarios cost
 * (0.9 s for the four-species consortium), so it happens on upload; above them
 * the core-metabolism overview is drawn instead and the full map stays one
 * explicit click away.
 */
const AUTO_DRAW_REACTION_LIMIT = 350;
const AUTO_DRAW_METABOLITE_LIMIT = 500;

export class Store {
  constructor(base = '.') {
    this.base = base;
    this.catalog = [];
    this.scenarios = [];
    // Benchmark communities from the literature. Metadata only: the papers these
    // come from use genome-scale models, which are reproduced headlessly by
    // tools/build-published-communities.mjs rather than drawn as a map.
    this.publishedCommunities = [];
    this.publishedCommunityId = null;
    // Manifests of the generated assets, one fetch each. A community with no
    // manifest has nothing downloadable and no map; the UI says so rather than
    // offering buttons that lead nowhere.
    this.publishedManifests = new Map();
    // What SteadyCom has been asked so far for the open community: the top of the
    // admissible range, where it came from, and the last growth rate solved at.
    this.steadyComState = null;
    this._steadyComSpec = null;
    this._steadyComRevision = 0;
    this._steadyComGeneration = 0;
    this._steadyComController = null;
    this.steadyComConfig = { fiber: null, parsimonious: false };
    this.runSteadyComTask = runSteadyComTask;
    this.zunigaStudy = null;
    this.zunigaPrepared = null;
    this.zunigaScenarioId = null;
    this.zunigaComparison = 'absolute';
    this.zunigaSolutions = new Map();
    this._zunigaGeneration = 0;
    this._zunigaPreviousView = null;
    this.loadZunigaStudy = loadZunigaStudy;
    this.models = new Map();      // file stem -> Model
    this.uploadedModelKeys = new Set();

    this.scenario = null;
    this.species = [];            // { id, file, abundance, enabled }
    this.objective = OBJECTIVES.EQUAL_GROWTH;
    this.productTarget = null;
    this.parsimonious = false;    // false = FBA, true = pFBA
    this.pfbaBiomassPercent = 100; // percentage of the primary optimum fixed in pFBA
    this.loopless = false;         // exact loopless FBA (MILP)
    this.fvaEnabled = false;
    this.fvaRanges = new Map();
    this.fvaProgress = { running: false, done: 0, total: 0, ms: 0 };
    this._fvaGeneration = 0;
    this._fvaTimer = null;
    this.medium = {};             // base metabolite -> uptake limit (positive)

    this.community = null;
    this.problem = null;
    this.map = null;              // EditableMap: what is drawn
    this.mapHistory = new MapHistory(100);
    this.modelEdits = new WeakMap();      // Model -> biological edits made to it
    this.modelEditsSaved = new WeakMap(); // Model -> edits written to a file
    this.savedWork = null;        // { context, scenario }: see unsavedWork()
    this.solution = null;
    this.scale = null;
    this.crossFeeding = { links: [], perMetabolite: [] };
    this.crossFeedingExplorer = { enabled:false, index:null, query:'', metabolite:'', producer:'', consumer:'', minFlux:0, selectedId:null, selectedAt:null };
    this.growth = new Map();
    this.overrides = new Map();   // merged reaction id -> { lower, upper }
    this.manualPathways = new Map(); // id -> { id, name, color, visible, reactionIds:Set }
    this.pathwaySettings = new Map(); // model pathway name -> { color?, visible? }
    this.omicsDataset = null;
    this.omicsSettings = {
      enabled: false,
      dataType: 'gene',
      mode: 'single',
      conditionA: null,
      conditionB: null,
      metric: 'log2fc',
      pseudocount: 1,
    };

    this.selectedReaction = null;
    this.selectedGeometry = null;
    this.selectedEntity = null;
    this.multiSelection = emptySelection();
    this.reactionSearchQuery = '';
    this.modelEditorSpeciesId = null;
    this.modelEditorMode = null;
    this.cellPlacementSpeciesId = null;
    this.compartmentPlacementKey = null;
    // The shape a newly drawn compartment gets: 'auto' is the one its kind has.
    this.compartmentShapePreset = 'auto';
    this.variability = null;
    this.layoutSettings = layoutPreset(DEFAULT_LAYOUT_PRESET);
    this.mapMode = 'full';
    this.view = {
      labels: 'auto',
      // How the next new text is set: the last size and font chosen.
      textStyle: { fontSize: TEXT_SIZE.default, fontFamily: DEFAULT_TEXT_FONT },
      scrollInput: 'auto',
      fluxColorStops: null,
      reactionColorMode: 'flux',
      // How reaction lines are drawn, each its own switch: width by flux,
      // grey dashes for what carries none, a gap where one passes under another.
      widthByFlux: false,
      dashNoFlux: true,
      lineHops: true,
      // What lines are coloured by when not by flux: 'pathway' or 'subsystem'.
      lineColorGroup: 'pathway',
      pathwayRibbonMode: 'off',
      pathwayRibbonWidth: 56,
      pathwayRibbonOpacity: 0.18,
      showCurrency: true,
      currencyLabels: true,
      showCrossFeeding: false,
      // Exchanged metabolites drawn as cross-feeding lines: the `crossFeedingTop`
      // most important (0 = all), unless a list was picked by hand.
      crossFeedingTop: 5,
      crossFeedingShown: null,
      dark: false,
      geometryEdit: false,
      multiSelect: false,
      textTool: false,
    };
    this.omics = { dataset:null, enabled:false, mode:'single', conditionA:null, conditionB:null, metric:'log2fc', pseudocount:1, scores:new Map(), colors:new Map(), scale:null, report:null, gimmeEnabled:false, gimmeThreshold:null, gimmeObjectiveFraction:0.9, gimmeResult:null };
    this.log = [];                // what the Logs tab lists: see addLog()
    this.status = { state: 'idle', message: '', solveMs: 0, layoutMs: 0 };
    this.reproducibilityReplay = null;
    // What the last map load had to do to make the file fit, for the status line.
    this.mapLoadNote = '';
    // A map opened before its model was loaded: see loadMapDocument().
    this.waitingMap = null;
    this.listeners = new Set();
  }

  /**
   * The drawn map under the name `layout`, which the renderer, the drag helpers
   * and the saved scenarios use. An EditableMap exposes the same shape, so the
   * alias costs nothing.
   */
  get layout() { return this.map; }
  set layout(value) { this.map = value; }

  get isZunigaStudy() { return this.scenario?.published === ZUNIGA_ID && Boolean(this.zunigaStudy); }

  /** Leave the Zuñiga case: forget its models and solutions and give back the view it changed. */
  resetZunigaStudy() {
    this._zunigaGeneration++;
    if (this._zunigaPreviousView) this.view.showCrossFeeding = this._zunigaPreviousView.showCrossFeeding;
    this._zunigaPreviousView = null;
    this.zunigaStudy = null;
    this.zunigaPrepared = null;
    this.zunigaScenarioId = null;
    this.zunigaComparison = 'absolute';
    this.zunigaSolutions = new Map();
  }

  /** Throw when the Zuñiga case is open, whose objective, medium and bounds are the authors' and stay fixed. */
  assertPublishedStudyConstraints() {
    if (this.isZunigaStudy) throw new Error('The Zuñiga case preserves the authors’ objective, medium, bounds and population coefficients. Choose a documented case scenario to change these conditions. Map geometry remains editable.');
  }

  /** Listen to store changes: `fn(store, reason)`. Returns the function that unsubscribes. */
  subscribe(fn) { this.listeners.add(fn); return () => this.listeners.delete(fn); }
  emit(reason = 'change') {
    this.trackWorkContext();
    for (const fn of this.listeners) fn(this, reason);
  }

  /*
   * The status line holds one message at a time, and the one before it is gone
   * the moment anything else happens. Every message that is set also goes to
   * the log, which is what the Logs tab lists, so an import note, a warning or
   * an error can still be read after the next solve has replaced it.
   */
  get status() { return this._status; }
  set status(value) {
    this._status = value;
    if (value?.message && ['ok', 'warn', 'error'].includes(value.state)) this.addLog(value.state, value.message);
  }

  /**
   * Add one line to the log, newest last. The same line twice in a row is one
   * entry with a count; so are back-to-back solves (`kind: 'solve'`), which a
   * slider drag produces by the dozen and which would push everything else out.
   */
  addLog(level, message, { kind = 'message' } = {}) {
    this.log ??= [];
    const last = this.log.at(-1);
    if (last && last.level === level && (last.message === message || (kind === 'solve' && last.kind === 'solve'))) {
      last.message = message;
      last.time = Date.now();
      last.count += 1;
      return last;
    }
    const entry = { time: Date.now(), level, message, kind, count: 1 };
    this.log.push(entry);
    if (this.log.length > LOG_LIMIT) this.log.splice(0, this.log.length - LOG_LIMIT);
    return entry;
  }

  clearLog() {
    this.log = [];
    this.emit('status');
  }

  /** Public contract used by UI code/tests to keep BUILD and MODEL edits separate. */
  editingOperationDomain(name) { return operationDomain(name); }
  assertMapOperation(name) { return assertOperationDomain(name, OPERATION_DOMAIN.MAP); }
  assertModelOperation(name) { return assertOperationDomain(name, OPERATION_DOMAIN.MODEL); }

  /** Load the solver, the model catalogue and the scenario registry, and every catalogued model. Resolves to the store. */
  async init() {
    // GLPK is optional: when it fails to load the bundled simplex takes over,
    // which is enough for every core-scale model shipped with the project.
    this.solverReady = await initGLPK().catch(() => false);
    const [index, scenarios] = await Promise.all([
      fetchJSON(`${this.base}/models/index.json`),
      fetchJSON(`${this.base}/models/scenarios.json`),
    ]);
    this.catalog = index.models;
    this.scenarios = scenarios.scenarios;
    this.publishedCommunities = scenarios.publishedCommunities ?? [];
    // Open on one that actually reproduces its paper. The list stays in the
    // registry's order, which follows the papers, but landing on a blocked entry
    // would show an empty result where there is a real one to show.
    this.publishedCommunityId = (
      this.publishedCommunities.find((c) => c.reproduction?.status === 'reproduced')
      ?? this.publishedCommunities[0]
    )?.id ?? null;
    await Promise.all(this.catalog.map((m) => this.loadModel(m.file.replace(/\.json$/, ''))));
    return this;
  }

  /** The model stored as `models/<name>.json`, fetched once and kept by name. */
  async loadModel(name) {
    if (this.models.has(name)) return this.models.get(name);
    const doc = await fetchJSON(`${this.base}/models/${name}.json`);
    const model = new Model(doc);
    this.models.set(name, model);
    return model;
  }

  /**
   * Load the registered Yeast9 scenario. The same path is used by the Scenario
   * dropdown and by the convenience button in Species & abundance.
   */
  async importYeast9() {
    const scenario = this.scenarios.find((item) => item.id === YEAST9_SCENARIO_ID) ?? {
      id: YEAST9_SCENARIO_ID,
      name: `Yeast9 ${YEAST9_SOURCE.version} — S. cerevisiae genome-scale`,
      description: `Official yeast-GEM ${YEAST9_SOURCE.version}.`,
      loader: 'yeast9',
      objective: OBJECTIVES.SUM_BIOMASS,
      species: [{ id: 'sce', model: YEAST9_MODEL_KEY, abundance: 1 }],
    };
    return this.useYeast9Scenario(scenario);
  }

  /** Download the official yeast-GEM release, build it and draw the curated Yeast9 preview without solving. */
  async useYeast9Scenario(scenario) {
    this.resetZunigaStudy();
    this.status = {
      ...this.status,
      state: 'busy',
      message: `Downloading official Yeast9 ${YEAST9_SOURCE.version}…`,
    };
    this.emit('status');

    const response = await fetch(YEAST9_SOURCE.url);
    if (!response.ok) {
      throw new Error(`Yeast9 ${YEAST9_SOURCE.version} download failed (HTTP ${response.status}). You can still download yeast-GEM.xml manually and use Load model(s).`);
    }

    const text = await response.text();
    const prepared = prepareImportedModels([{ name: YEAST9_SOURCE.filename, text }]);
    const entry = prepared.entries[0];
    if (!entry) throw new Error('Yeast9 import produced no model.');

    for (const key of this.uploadedModelKeys) this.models.delete(key);
    this.uploadedModelKeys.clear();
    this.models.set(YEAST9_MODEL_KEY, entry.model);
    this.uploadedModelKeys.add(YEAST9_MODEL_KEY);

    this.scenario = scenario;
    this.objective = scenario.objective || OBJECTIVES.SUM_BIOMASS;
    this.productTarget = scenario.productTarget || null;
    this.parsimonious = Boolean(scenario.parsimonious ?? false);
    this.pfbaBiomassPercent = normalizePfbaPercent(scenario.pfbaBiomassPercent ?? 100);
    this.loopless = false;
    this.fvaEnabled = false;
    this.fvaRanges = new Map();
    this.fvaProgress = { running: false, done: 0, total: 0, ms: 0 };
    this.medium = { ...prepared.medium };
    this.overrides.clear();
    this.manualPathways.clear();
    this.pathwaySettings.clear();
    this.clearOmics({ emit: false });
    this.selectedReaction = null;
    this.selectedGeometry = null;
    this.selectedEntity = null;
    this.multiSelection = emptySelection();
    this.variability = null;

    const speciesId = scenario.species?.[0]?.id || 'sce';
    this.species = [{
      id: speciesId,
      file: YEAST9_MODEL_KEY,
      abundance: 1,
      enabled: true,
    }];
    this.modelEditorSpeciesId = speciesId;
    this.modelEditorMode = null;
    this.cellPlacementSpeciesId = speciesId;
    this.compartmentPlacementKey = null;

    // Build the biological model and its 14-compartment shell, but do not solve
    // the 4,000-reaction LP automatically. The preview below is map-only.
    await this.rebuild({ relayout: true, contents: 'none', solve: false });
    // The generic automatic canvas is sized for all 4,105 reactions. The
    // curated scenario draws only a representative subset, so use a compact
    // publication-like canvas where the 14 compartments remain legible.
    this.map = EditableMap.emptyFromCommunity(this.community, {
      width: 2100,
      height: 1550,
      ...layoutEngineOptions(this.layoutSettings),
    });
    this.mapHistory.clear();
    const preview = this.populateYeast9Preview(entry.model, speciesId);

    this.status = {
      ...this.status,
      state: 'ok',
      message: `Loaded Yeast9 ${YEAST9_SOURCE.version}: ${entry.summary.reactions} reactions, ${entry.summary.metabolites} metabolites, ${entry.summary.compartments} compartments · ${preview.placed} representative reactions drawn across ${preview.coveredCompartments}/${YEAST9_PREVIEW_COMPARTMENTS.length} compartments. Run Analysis when you want FBA.`,
    };
    this.settleOpenedWork();
    this.drawWaitingMap();
    this.emit('rebuild');
    return { ...prepared, preview };
  }

  /**
   * Draw a curated, readable Yeast9 map rather than a genome-scale hairball.
   * Reactions are spread through the free area of their biological compartment,
   * avoiding child organelles whenever possible.
   */
  populateYeast9Preview(model, speciesId = 'sce') {
    if (!this.map || !this.community || !model) {
      return { placed: 0, coveredCompartments: 0, missing: [] };
    }

    const specs = yeast9PreviewSpecs(model);
    const byCompartment = new Map();
    for (const spec of specs) {
      if (!byCompartment.has(spec.compartment)) byCompartment.set(spec.compartment, []);
      byCompartment.get(spec.compartment).push(spec);
    }

    let placed = 0;
    const covered = new Set();
    const missing = [];

    for (const [compartmentId, group] of byCompartment) {
      const anchors = yeastPreviewAnchors(this.map, speciesId, compartmentId, group.length);
      group.forEach((spec, index) => {
        const mergedId = namespaced(speciesId, spec.reactionId);
        const result = this.map.addReaction(this.community, mergedId, {
          anchor: anchors[index] ?? null,
        });
        if (result) {
          placed += 1;
          covered.add(compartmentId);
        } else {
          missing.push(spec.reactionId);
        }
      });
    }

    this.mapHistory.clear();
    return { placed, coveredCompartments: covered.size, missing };
  }

  /**
   * Replace the current community with one built from user-supplied COBRA JSON
   * files. Multiple files become multiple species/cells.
   */
  async importModelFiles(files) {
    const prepared = prepareImportedModels(files);
    this.resetZunigaStudy();

    for (const key of this.uploadedModelKeys) this.models.delete(key);
    this.uploadedModelKeys.clear();

    for (const entry of prepared.entries) {
      this.models.set(entry.modelKey, entry.model);
      this.uploadedModelKeys.add(entry.modelKey);
    }

    const scenario = {
      id: 'uploaded-models',
      name: prepared.entries.length === 1
        ? prepared.entries[0].model.name
        : 'Imported community',
      description: prepared.entries.length === 1
        ? `Model loaded from ${prepared.entries[0].filename}`
        : `${prepared.entries.length} models loaded from files`,
      objective: OBJECTIVES.EQUAL_GROWTH,
      parsimonious: false,
      pfbaBiomassPercent: 100,
      loopless: false,
      fvaEnabled: false,
      medium: prepared.medium,
      species: prepared.entries.map((entry) => ({
        id: entry.speciesId,
        model: entry.modelKey,
        abundance: entry.abundance,
        enabled: true,
      })),
      uploaded: true,
    };

    this.scenarios = this.scenarios.filter((s) => !['uploaded-json', scenario.id].includes(s.id));
    this.scenarios.push(scenario);
    this.scenario = scenario;
    this.objective = OBJECTIVES.EQUAL_GROWTH;
    this.productTarget = null;
    this.parsimonious = false;
    this.pfbaBiomassPercent = 100;
    this.loopless = false;
    this.fvaEnabled = false;
    this.fvaRanges = new Map();
    this.fvaProgress = { running: false, done: 0, total: 0, ms: 0 };
    this.medium = { ...prepared.medium };
    this.overrides.clear();
    this.manualPathways.clear();
    this.pathwaySettings.clear();
    this.clearOmics({ emit: false });
    this.selectedReaction = null;
    this.selectedGeometry = null;
    this.selectedEntity = null;
    this.multiSelection = emptySelection();
    this.variability = null;
    this.species = scenario.species.map((s) => ({
      id: s.id,
      file: s.model,
      abundance: s.abundance,
      enabled: true,
    }));
    this.modelEditorSpeciesId = this.species[0]?.id ?? null;
    this.modelEditorMode = null;
    this.cellPlacementSpeciesId = this.species.find((sp) => sp.enabled !== false)?.id ?? null;
    this.compartmentPlacementKey = null;

    const totalReactions = prepared.entries.reduce((n, x) => n + x.summary.reactions, 0);
    const totalMetabolites = prepared.entries.reduce((n, x) => n + x.summary.metabolites, 0);
    const autoSolve = totalReactions <= AUTO_SOLVE_REACTION_LIMIT;
    // Crowding is per cell, so it is the largest member that decides, not the sum.
    const drawable = prepared.entries.every((x) =>
      x.summary.reactions <= AUTO_DRAW_REACTION_LIMIT
      && x.summary.metabolites <= AUTO_DRAW_METABOLITE_LIMIT);

    await this.rebuild({ relayout: true, contents: drawable ? 'all' : 'none', solve: autoSolve });

    // Bare cells and compartments with nothing inside them read as a failed
    // import. So draw what can be drawn quickly, and where that is the whole
    // model say so, so nobody goes looking for the rest.
    let drawn;
    if (drawable) {
      this.mapMode = 'full';
      drawn = ` Drawn in full: ${this.map?.reactions?.size ?? 0} reactions.`;
    } else if (this.showCoreMetabolismOverview({ recordHistory: false })) {
      drawn = ' Drawing central carbon metabolism only — this model is too large to lay out in full automatically; use Map options ▸ Full map for everything.';
    } else {
      this.mapMode = 'empty';
      drawn = ' The drawing was left empty: this model is too large to lay out automatically. Build it up from Map structure, or use Map options ▸ Full map.';
    }

    this.status = {
      ...this.status,
      state: 'ok',
      message: `Imported ${prepared.entries.length} model(s): ${totalReactions} reactions, ${totalMetabolites} metabolites.${autoSolve ? '' : ' Analysis was deferred for this genome-scale model; use Run Analysis when needed.'}${drawn}`,
    };
    this.settleOpenedWork();
    this.drawWaitingMap();
    this.emit('rebuild');

    return prepared;
  }

  refreshCrossFeedingExplorer() {
    if(!this.community||this.solution?.status!=='optimal'){this.crossFeedingExplorer.index=null;return;}
    this.crossFeedingExplorer.index=buildCrossFeedingIndex(this.community,this.solution.fluxes,{threshold:1e-6, raw:this.solution.steadyCom || this._jointProjectionSpec ? this.crossFeeding : null});
    const visible=this.crossFeedingExplorerEvents();
    if(this.crossFeedingExplorer.selectedId&&!visible.some(x=>x.id===this.crossFeedingExplorer.selectedId))this.crossFeedingExplorer.selectedId=null;
  }

  crossFeedingExplorerEvents() { return filterCrossFeedingIndex(this.crossFeedingExplorer.index,this.crossFeedingExplorer); }

  setCrossFeedingExplorer(options={}) {
    Object.assign(this.crossFeedingExplorer,options);
    if(options.enabled===true||!this.crossFeedingExplorer.index)this.refreshCrossFeedingExplorer();
    this.view.showCrossFeeding=true; this.emit('cross-feeding-explorer');
  }

  /**
   * Pick the event a cross-feeding line on the map stands for, named by its
   * producer, metabolite and consumer, with the map point that was clicked (the
   * name tag is pinned there). Picking the picked line again unpicks it.
   */
  toggleCrossFeedingLink({ from, base, to }, at = null) {
    if (!this.crossFeedingExplorer.index) this.refreshCrossFeedingExplorer();
    const event = this.crossFeedingExplorer.index?.events.find((e) => e.from === from && e.base === base && e.to === to);
    if (!event) return null;
    if (this.crossFeedingExplorer.selectedId === event.id) {
      this.clearCrossFeedingPick();
      return null;
    }
    this.crossFeedingExplorer.selectedAt = at;
    return this.selectCrossFeedingEvent(event.id);
  }

  /** The metabolites species pass to each other, most important first (protons and water left out). */
  crossFeedingMetabolites() {
    return exchangedMetabolites(this.crossFeeding?.links, (base) => !isDrawnCrossFeedingBase(base));
  }

  /** The bases whose cross-feeding lines are drawn. */
  crossFeedingVisibleBases() {
    if (Array.isArray(this.view.crossFeedingShown)) return new Set(this.view.crossFeedingShown);
    const all = this.crossFeedingMetabolites();
    const top = this.view.crossFeedingTop;
    return new Set((top > 0 ? all.slice(0, top) : all).map((entry) => entry.base));
  }

  /** Draw the `n` most important exchanged metabolites (0: all of them). */
  setCrossFeedingTop(n) {
    const top = Math.max(0, Math.round(Number(n) || 0));
    this.setView({ crossFeedingTop: top, crossFeedingShown: null });
  }

  /** Tick or untick one metabolite; from then on the list is the user's own. */
  toggleCrossFeedingMetabolite(base, on) {
    const shown = this.crossFeedingVisibleBases();
    if (on) shown.add(base);
    else shown.delete(base);
    this.setView({ crossFeedingShown: [...shown] });
  }

  /** Unpick the picked cross-feeding event, leaving the explorer as it is. */
  clearCrossFeedingPick() {
    if (!this.crossFeedingExplorer.selectedId) return;
    this.crossFeedingExplorer.selectedId = null;
    this.crossFeedingExplorer.selectedAt = null;
    this.emit('cross-feeding-explorer');
  }

  selectCrossFeedingEvent(id) {
    this.crossFeedingExplorer.enabled=true;this.crossFeedingExplorer.selectedId=id;
    const e=this.crossFeedingExplorer.index?.events.find(x=>x.id===id);
    if(e?.producerTransfer)this.selectedReaction=e.producerTransfer;
    this.emit('cross-feeding-explorer'); return e;
  }

  /** Backwards-compatible alias for callers from pre-SBML releases. */
  async importModelJSONFiles(files) { return this.importModelFiles(files); }

  // Community omics / metatranscriptomics

  /** Read a metatranscriptomics CSV, map it onto the community's GPR rules and show it. Returns the parsed data with its mapping report. */
  importMetatranscriptomics(text, filename = 'metatranscriptomics.csv') {
    const parsed = parseMetatranscriptomics(text, { source: filename });
    this.omics.dataset = parsed.dataset;
    this.omics.enabled = true;
    this.omics.conditionA = parsed.dataset.conditions[0] ?? null;
    this.omics.conditionB = parsed.dataset.conditions[1] ?? parsed.dataset.conditions[0] ?? null;
    this.omics.report = communityOmicsMappingReport(this.community, this.species, this.models, parsed.dataset);
    this.recomputeCommunityOmics();
    this.status = { ...this.status, state:'ok', message:`Metatranscriptomics: ${parsed.dataset.rowCount} genes/MAG, ${Math.round(this.omics.report.reactionCoverage*100)}% of GPR reactions mapped.` };
    this.emit('omics');
    return { ...parsed, report:this.omics.report };
  }

  /**
   * Fit the fluxes to condition A of the loaded metatranscriptomics with GIMME.
   * `options` patches the omics settings first (threshold, objective fraction).
   * Installs the GIMME solution and returns the solver result.
   */
  runCommunityGIMME(options = {}) {
    this.assertPublishedStudyConstraints();
    Object.assign(this.omics, options);
    if(!this.omics.dataset) throw new Error('Load metatranscriptomics data first.');
    this.cancelSteadyCom({ silent: true });
    if (this.solution?.steadyCom) this.refreshCompositionProblem();
    const condition=this.omics.conditionA;
    const expression=reactionOmicsScores(this.community,this.species,this.models,this.omics.dataset,condition);
    const result=runGIMME(this.problem,expression,{threshold:this.omics.gimmeThreshold,objectiveFraction:this.omics.gimmeObjectiveFraction});
    this.omics.gimmeResult=result; this.omics.gimmeEnabled=result.status==='optimal';
    this.applySolution(result, { allowFVA:false });
    if(result.status==='optimal'){
      this.status={...this.status,state:'ok',message:`GIMME ${condition}: objective ${(100*result.objectiveFraction).toFixed(0)}%, threshold ${Number(result.threshold).toPrecision(4)}, inconsistency ${Number(result.inconsistencyScore).toPrecision(4)}. Duals: contextual GIMME LP.`};
    } else this.status={...this.status,state:'error',message:`GIMME found no solution: ${result.message||result.status}`};
    this.emit('solution'); return result;
  }

  disableGIMME() { this.omics.gimmeEnabled=false; this.omics.gimmeResult=null; return this.solve(); }

  setCommunityOmics(options = {}) {
    Object.assign(this.omics, options);
    this.recomputeCommunityOmics();
    this.emit('omics');
  }

  /** Reaction scores and colours from the omics data: condition A alone, or A against B in compare mode. */
  recomputeCommunityOmics() {
    const o=this.omics, d=o.dataset;
    if(!d || !o.enabled){o.scores=new Map();o.colors=new Map();o.scale=null;return;}
    const a=reactionOmicsScores(this.community,this.species,this.models,d,o.conditionA);
    if(o.mode==='compare' && o.conditionB){
      const b=reactionOmicsScores(this.community,this.species,this.models,d,o.conditionB);
      o.scores=compareReactionScores(a,b,{metric:o.metric,pseudocount:o.pseudocount});
      o.scale=divergingOmicsColors(o.scores,{dark:this.view.dark});
    }else{
      o.scores=a; o.scale=sequentialOmicsColors(a,{dark:this.view.dark});
    }
    o.colors=o.scale.colors;
    o.report=communityOmicsMappingReport(this.community,this.species,this.models,d);
  }

  // Biological model editor

  modelEditorSpecies() {
    const preferred = this.species.find((sp) => sp.id === this.modelEditorSpeciesId);
    return preferred ?? this.species[0] ?? null;
  }

  modelEditorModel() {
    const sp = this.modelEditorSpecies();
    return sp ? this.models.get(sp.file) ?? null : null;
  }

  setModelEditorSpecies(id) {
    if (!this.species.some((sp) => sp.id === id)) return null;
    this.modelEditorSpeciesId = id;
    this.emit('model-editor');
    return this.modelEditorSpecies();
  }

  setModelEditorMode(mode) {
    if (mode) this.assertPublishedStudyConstraints();
    this.modelEditorMode = ['metabolite', 'reaction'].includes(mode) ? mode : null;
    this.emit('model-editor');
    return this.modelEditorMode;
  }

  /**
   * Add a metabolite to the model of the model editor's species and rebuild the
   * community. A MODEL operation. Returns the new metabolite.
   */
  async createModelMetabolite(spec) {
    this.assertPublishedStudyConstraints();
    this.assertModelOperation('createModelMetabolite');
    const sp = this.modelEditorSpecies();
    const model = this.modelEditorModel();
    if (!sp || !model) throw new Error('Select a species/model.');

    const metabolite = addMetaboliteToModel(model, spec);
    this.modelEdits.set(model, (this.modelEdits.get(model) ?? 0) + 1);
    await this.rebuild({ relayout: false });
    this.status = {
      ...this.status,
      state: 'ok',
      message: `Metabolite ${metabolite.id} created in ${sp.id}.`,
    };
    this.emit('model-edit');
    return metabolite;
  }

  /**
   * Add a reaction to the model of the model editor's species and rebuild the
   * community, drawing and selecting it too with `addToMap`. A MODEL operation.
   * Returns { reaction, mergedId, mapResult }.
   */
  async createModelReaction(spec, { addToMap = false } = {}) {
    this.assertPublishedStudyConstraints();
    this.assertModelOperation('createModelReaction');
    const sp = this.modelEditorSpecies();
    const model = this.modelEditorModel();
    if (!sp || !model) throw new Error('Select a species/model.');

    const reaction = addReactionToModel(model, spec);
    this.modelEdits.set(model, (this.modelEdits.get(model) ?? 0) + 1);
    await this.rebuild({ relayout: false });

    const mergedId = `${sp.id}@${reaction.id}`;
    let mapResult = null;
    if (addToMap && this.map && this.community?.merged.reactions.has(mergedId)) {
      mapResult = this.addMapReaction(mergedId);
      if (mapResult) this.select(mergedId);
    }

    this.status = {
      ...this.status,
      state: 'ok',
      message: `Reaction ${reaction.id} created in ${sp.id}${mapResult ? ' and added to the map' : ''}.`,
    };
    this.emit('model-edit');
    return { reaction, mergedId, mapResult };
  }

  /** A species' model as COBRA JSON text, by default the model editor's species. */
  modelJSON(speciesId = this.modelEditorSpeciesId) {
    const sp = this.species.find((item) => item.id === speciesId) ?? this.modelEditorSpecies();
    if (!sp) throw new Error('Select a species to export.');
    const model = this.models.get(sp.file);
    if (!model) throw new Error(`The model for ${sp.id} was not found.`);
    return JSON.stringify(model.toJSON(), null, 2);
  }

  /**
   * The selected species as SBML L3V1 + fbc-v2, the format cobrapy and the
   * COBRA Toolbox read. Built from the model in memory, so it covers whatever
   * the user has edited or imported, not just what shipped.
   */
  modelSBML(speciesId = this.modelEditorSpeciesId) {
    const sp = this.species.find((item) => item.id === speciesId) ?? this.modelEditorSpecies();
    if (!sp) throw new Error('Select a species to export.');
    const model = this.models.get(sp.file);
    if (!model) throw new Error(`The model for ${sp.id} was not found.`);
    return toSBML(model);
  }

  /**
   * A published member's full model, converted to SBML on demand.
   *
   * The XML runs to roughly three times its JSON, and is derived from it
   * mechanically, so the repository carries the JSON and this makes the SBML
   * when somebody asks for it. Over file:// the browser refuses the fetch (the
   * single-file build cannot read the files beside it), and the caller says
   * what to do about it rather than failing silently.
   */
  async publishedMemberSBML(communityId, memberId) {
    if (communityId === ZUNIGA_ID) {
      const model = this.models.get(`published/${ZUNIGA_ID}/model-${memberId}`);
      if (!model) throw new Error('Open the Zuñiga case to privately load and verify this model first.');
      return { xml: toSBML(model), name: `${communityId}-${memberId}` };
    }
    const manifest = await this.publishedManifest(communityId);
    const member = manifest?.members?.find((m) => m.id === memberId);
    if (!member) throw new Error(`${memberId} has no model file in this build.`);
    const doc = await fetchJSON(`${this.base}/models/published/${communityId}/${member.model}`);
    return { xml: toSBML(new Model(doc)), name: `${communityId}-${memberId}` };
  }

  /** The reproducibility bundle of the current FBA-family analysis. Throws for a fixed-µ SteadyCom solution. */
  async reproducibilityBundle() {
    if (this.solution?.steadyCom) throw new Error('Fixed-μ SteadyCom replay is saved through Scenario JSON. This reproducibility bundle supports ordinary FBA analyses; export the scenario and flux CSV for this solution.');
    return createReproducibilityBundle(this);
  }

  async methodsProvenanceReport() { const bundle=await this.reproducibilityBundle(); return { bundle, text:methodsProvenanceReport(bundle) }; }

  async verifyReproducibilityBundle(bundle) { return verifyReproducibilityBundle(bundle); }

  /**
   * Open a reproducibility bundle: check its SHA-256 integrity, install its
   * models, constraints, map and omics, solve again with the archived method
   * and compare the result with the archived solution. Resolves to the
   * integrity check with the replay comparison added as `replay`.
   */
  async importReproducibilityBundle(bundle) {
    const check=await verifyReproducibilityBundle(bundle);
    if(!check.valid) throw new Error('The bundle fails SHA-256 verification; it may have been modified or be corrupt.');
    if (bundle.study?.id === ZUNIGA_ID) return this.importZunigaBundle(bundle, check);
    this.resetZunigaStudy();
    for(const key of this.uploadedModelKeys)this.models.delete(key); this.uploadedModelKeys.clear();
    const species=[];
    for(const sp of bundle.community?.species??[]){
      const doc=bundle.models?.[sp.id]; if(!doc)continue;
      const key=`repro-${sp.id}`;this.models.set(key,new Model(doc));this.uploadedModelKeys.add(key);
      species.push({...sp,file:key});
    }
    if(!species.length)throw new Error('The bundle contains no reproducible models.');

    this.scenario={id:'reproducibility-bundle',name:bundle.community?.name??'Reproduced analysis',objective:bundle.community?.objective,species};
    this.species=species;this.objective=bundle.community?.objective??OBJECTIVES.EQUAL_GROWTH;this.productTarget=bundle.community?.productTarget??null;
    this.medium={...(bundle.constraints?.medium??{})};this.overrides=new Map(Object.entries(bundle.constraints?.reactionOverrides??{}));
    this.parsimonious=Boolean(bundle.analysis?.parsimonious);this.pfbaBiomassPercent=bundle.analysis?.pfbaBiomassPercent??100;
    this.loopless=Boolean(bundle.analysis?.loopless);
    // FVA is replayed only after the primary numerical solution has been
    // compared; its asynchronous ranges are restored from the archive below.
    this.fvaEnabled=false;

    await this.rebuild({relayout:true});

    // Restore the exact archived drawing after the biological model exists.
    if(bundle.map){
      this.map=EditableMap.fromJSON(bundle.map);
      this.mapHistory.clear();
    }

    // Rehydrate omics Map structures and settings before replaying GIMME.
    const archivedOmics=bundle.omics;
    if(archivedOmics){
      const values=new Map(Object.entries(archivedOmics.values??{}));
      this.omics.dataset={
        source:archivedOmics.source??'reproducibility-bundle',
        dataType:archivedOmics.dataType??'omics',
        conditions:[...(archivedOmics.conditions??[])],
        values,
        rowCount:values.size,
      };
      Object.assign(this.omics,archivedOmics.settings??{});
      this.omics.gimmeResult=null;
      this.recomputeCommunityOmics();
    }else{
      this.omics={...this.omics,dataset:null,enabled:false,scores:new Map(),colors:new Map(),scale:null,report:null,gimmeEnabled:false,gimmeResult:null};
    }

    const method=bundle.analysis?.method??'FBA';
    if(method==='GIMME'){
      if(!this.omics.dataset)throw new Error('The bundle declares GIMME but lacks the omics dataset needed to reproduce it.');
      this.omics.gimmeEnabled=false;
      this.omics.gimmeThreshold=bundle.analysis?.gimme?.threshold??this.omics.gimmeThreshold;
      this.omics.gimmeObjectiveFraction=bundle.analysis?.gimme?.objectiveFraction??this.omics.gimmeObjectiveFraction??0.9;
      this.omics.conditionA=bundle.analysis?.gimme?.condition??this.omics.conditionA;
      this.runCommunityGIMME();
    }else{
      this.omics.gimmeEnabled=false;
      this.solve({parsimonious:method==='pFBA',silent:true});
    }

    const tolerances=bundle.replay?.tolerances??{absolute:1e-7,relative:1e-6};
    const replay=compareReplayedSolution(bundle.solution,this.solution,tolerances);
    check.replay=replay;

    // Restore archived FVA ranges immediately. A user can explicitly recompute
    // them later; this avoids racing asynchronous FVA against replay validation.
    if(bundle.analysis?.fvaEnabled && bundle.solution?.fva){
      this.fvaEnabled=true;
      this.fvaRanges=new Map(Object.entries(bundle.solution.fva));
      this.fvaProgress={running:false,done:this.fvaRanges.size,total:this.fvaRanges.size,ms:0};
    }else{
      this.fvaEnabled=false;this.fvaRanges=new Map();this.fvaProgress={running:false,done:0,total:0,ms:0};
    }

    this.reproducibilityReplay={integrity:check,replay,archivedAt:bundle.metadata?.createdAt??null,restored:{map:Boolean(bundle.map&&this.map),omicsExpected:Boolean(bundle.omics),omics:Boolean(bundle.omics&&this.omics.dataset)}};
    const verdict=replay.pass?'numerical replay OK':'WARNING: numerical differences';
    this.settleOpenedWork();
    this.status={...this.status,state:replay.pass?'ok':'error',message:`Bundle restored · ${verdict} · max Δflux ${replay.fluxes.maxAbsError.toExponential(2)} · SHA-256 ${bundle.integrity.payloadHash.slice(0,12)}…`};
    this.drawWaitingMap();
    this.emit('rebuild'); return check;
  }

  /** Load a named scenario and rebuild everything. */
  async useScenario(id) {
    this.resetZunigaStudy();
    this.invalidateSteadyCom();
    this.steadyComConfig = { fiber: null, parsimonious: false };
    this.publishedCommunityId = null;
    const scenario = this.scenarios.find((s) => s.id === id) || this.scenarios[0];
    if (scenario?.loader === 'yeast9') return this.useYeast9Scenario(scenario);
    this.scenario = scenario;
    this.objective = scenario.objective || OBJECTIVES.EQUAL_GROWTH;
    this.productTarget = scenario.productTarget || null;
    this.parsimonious = Boolean(scenario.parsimonious ?? scenario.pfba ?? false);
    this.pfbaBiomassPercent = normalizePfbaPercent(
      scenario.pfbaBiomassPercent
      ?? (scenario.pfbaFraction !== undefined ? Number(scenario.pfbaFraction) * 100 : 100)
    );
    this.loopless = Boolean(scenario.loopless ?? scenario.looplessFba ?? false);
    if (this.loopless) this.parsimonious = false;
    this.fvaEnabled = this.loopless
      ? false
      : Boolean(scenario.fvaEnabled ?? scenario.fva ?? false);
    this.fvaRanges = new Map();
    this.fvaProgress = { running: false, done: 0, total: 0, ms: 0 };
    this.medium = { ...scenario.medium };
    this.overrides.clear();
    this.manualPathways.clear();
    this.pathwaySettings.clear();
    this.clearOmics({ emit: false });
    this.selectedReaction = null;
    this.selectedEntity = null;
    this.variability = null;
    this.species = scenario.species.map((s) => ({
      id: s.id,
      file: s.model,
      abundance: s.abundance ?? 1 / scenario.species.length,
      enabled: s.enabled !== false,
    }));
    this.modelEditorSpeciesId = this.species[0]?.id ?? null;
    this.modelEditorMode = null;
    await this.rebuild({ relayout: true });
    this.settleOpenedWork();
    this.drawWaitingMap();
    return this;
  }

  /**
   * The generated assets for a published community: the per-member models, the
   * medium, the bounds and the map excerpts. Fetched once and remembered, with
   * null meaning "this community has none", which is the normal case for the
   * ones whose sources could not be retrieved.
   */
  async publishedManifest(id) {
    if (this.publishedManifests.has(id)) return this.publishedManifests.get(id);
    let manifest = null;
    try {
      manifest = await fetchJSON(`${this.base}/models/published/${id}/manifest.json`);
    } catch {
      manifest = null;
    }
    this.publishedManifests.set(id, manifest);
    return manifest;
  }

  /** Author models are fetched privately; the drawing never becomes the LP. */
  async useZunigaStudy({ study = null, scenarioId = 'mutant-coculture' } = {}) {
    this.invalidateSteadyCom();
    this.resetZunigaStudy();
    const generation = this._zunigaGeneration;
    const assertCurrent = () => { if (generation !== this._zunigaGeneration) { const error = new Error('The Zuñiga load was superseded by another scenario.'); error.name = 'AbortError'; throw error; } };
    this.publishedCommunityId = ZUNIGA_ID;
    const record = this.publishedCommunities.find(c => c.id === ZUNIGA_ID);
    this.scenario = { id: `published:${ZUNIGA_ID}`, name: record?.name ?? 'Zuñiga et al. (2020) — pheA rescue', published: ZUNIGA_ID };
    this.status = { ...this.status, state: 'busy', message: 'Loading the authors’ complete models and verifying their SHA-256 hashes…' };
    this.emit('status');
    try {
      const manifest = study?.manifest ?? await this.publishedManifest(ZUNIGA_ID);
      if (!manifest) throw new Error('The Zuñiga case manifest is unavailable.');
      if (!glpkAvailable() && !await initGLPK()) throw new Error('GLPK is required to calculate the complete Zuñiga source models.');
      const loaded = study ?? await this.loadZunigaStudy(manifest, {
        onProgress: message => {
          if (generation !== this._zunigaGeneration) return;
          this.status = { ...this.status, state: 'busy', message: typeof message === 'string' ? message : (message?.message ?? 'Loading the Zuñiga models…') };
          this.emit('status');
        },
      });
      assertCurrent();
      this.zunigaStudy = loaded;
      this.publishedManifests.set(ZUNIGA_ID, manifest);
      this.species = ['Ec', 'Se'].map(id => {
        const file = `published/${ZUNIGA_ID}/model-${id}`;
        this.models.set(file, this.zunigaStudy.models[id]);
        return { id, file, abundance: 1, enabled: true };
      });
      this.modelEditorSpeciesId = 'Ec';
      this.modelEditorMode = null;
      this.objective = 'published-original';
      this._zunigaPreviousView = { showCrossFeeding: this.view.showCrossFeeding };
      this.view.showCrossFeeding = false;
      this.view.reactionColorMode = 'flux';
      this.view.pathwayRibbonMode = 'off';
      this.productTarget = null;
      this.parsimonious = false;
      this.loopless = false;
      this.fvaEnabled = false;
      this.cancelFVA({ clear: true, emit: false });
      this.medium = {};
      this.overrides.clear();
      this.manualPathways.clear();
      this.pathwaySettings.clear();
      this.clearOmics({ emit: false });
      this._jointProjectionSpec = null;
      this.steadyComConfig = { fiber: null, parsimonious: false };
      this.selectedReaction = null;
      this.selectedGeometry = null;
      this.selectedEntity = null;
      this.multiSelection = emptySelection();
      this.variability = null;
      for (const scenario of ZUNIGA_SCENARIOS) {
        this.status = { ...this.status, state: 'busy', message: `Calculating ${scenario.name} on the complete model…` };
        this.emit('status');
        await frame();
        assertCurrent();
        const prepared = createZunigaScenario(this.zunigaStudy, scenario.id);
        const result = solveZunigaScenario(prepared);
        if (result.status !== 'optimal') throw new Error(`${scenario.name}: ${result.message ?? result.status}`);
        result.zuniga.growth = Object.fromEntries(prepared.community.growthRates(result.fluxes));
        result.zuniga.crossFeeding = prepared.community.crossFeeding(result.fluxes);
        this.zunigaSolutions.set(scenario.id, result);
      }
      const drawing = await fetchJSON(`${this.base}/models/published/${ZUNIGA_ID}/${manifest.map ?? 'map.json'}`);
      assertCurrent();
      this.map = EditableMap.fromJSON(drawing.map ?? drawing);
      this.mapMode = 'published-case';
      this.mapHistory.clear();
      this.useZunigaScenario(scenarioId, { emit: false });
      this.settleOpenedWork();
      this.emit('rebuild');
      return this;
    } catch (error) {
      if (generation !== this._zunigaGeneration) throw error;
      this.resetZunigaStudy();
      this.community = null;
      this.problem = null;
      this.map = null;
      this.solution = null;
      this.scale = null;
      this.growth = new Map();
      this.crossFeeding = { links: [], perMetabolite: [] };
      this.crossFeedingExplorer.index = null;
      this.mapHistory.clear();
      this.selectedReaction = null;
      this.selectedEntity = null;
      this.status = { ...this.status, state: 'error', message: error.message };
      this.emit('rebuild');
      throw error;
    }
  }

  /** Switch the open Zuñiga case to one of its scenarios and show that scenario's solution. */
  useZunigaScenario(id, { emit = true } = {}) {
    if (!this.isZunigaStudy) throw new Error('Open the Zuñiga published case first.');
    const scenario = ZUNIGA_SCENARIOS.find(item => item.id === id);
    if (!scenario) throw new Error(`Unknown Zuñiga case scenario: ${id}`);
    this.zunigaPrepared = createZunigaScenario(this.zunigaStudy, id);
    this.zunigaScenarioId = id;
    this.community = this.zunigaPrepared.community;
    this.problem = this.zunigaPrepared.problem;
    this.defaultBounds = this.problem.snapshotBounds();
    this.overrides.clear();
    this.scenario = { ...this.scenario, caseScenario: id };
    this.scenario.name = `${this.publishedCommunities.find(c => c.id === ZUNIGA_ID)?.name ?? 'Zuñiga et al. (2020)'} — ${scenario.name}`;
    this.selectedReaction = null;
    this.selectedEntity = null;
    const result = this.zunigaSolutions.get(id) ?? solveZunigaScenario(this.zunigaPrepared);
    result.zuniga.growth = Object.fromEntries(this.community.growthRates(result.fluxes));
    result.zuniga.crossFeeding = this.community.crossFeeding(result.fluxes);
    this.zunigaSolutions.set(id, result);
    this.applySolution(result, { allowFVA: false });
    this.status = { ...this.status, state: result.status === 'optimal' ? 'ok' : 'error', message: `${scenario.name} · exact source constraints · stationary FBA adaptation of the authors’ community model.` };
    if (emit) this.emit('solution');
    return result;
  }

  /** Show the Zuñiga fluxes as they are ('absolute') or minus the WT monoculture ('difference'). */
  setZunigaComparison(mode) {
    if (!this.isZunigaStudy) return;
    if (!['absolute', 'difference'].includes(mode)) throw new Error('The comparison must be absolute or difference.');
    this.zunigaComparison = mode;
    if (mode === 'difference') this.view.reactionColorMode = 'flux';
    this.refreshFluxScale();
    this.emit('view');
  }

  /** The fluxes the map shows: the solution's, or their difference from the baseline in the Zuñiga difference view. */
  displayFluxes() {
    const actual = this.solution?.status === 'optimal' ? this.solution.fluxes : new Map();
    if (!this.isZunigaStudy || this.zunigaComparison !== 'difference') return actual;
    const baseline = this.zunigaSolutions.get('wt-monoculture')?.fluxes ?? new Map();
    return new Map([...new Set([...actual.keys(), ...baseline.keys()])].map(id => [id, (actual.get(id) ?? 0) - (baseline.get(id) ?? 0)]));
  }

  zunigaState() {
    return this.isZunigaStudy ? { id: ZUNIGA_ID, scenarioId: this.zunigaScenarioId, comparison: this.zunigaComparison, fluxReference: this.zunigaStudy.manifest.fluxReference ?? 0.1, sourceHashes: this.zunigaStudy.sourceHashes } : null;
  }

  /** Throw unless a saved Zuñiga state matches the open case: scenario, comparison, flux scale and source hashes. */
  validateZunigaState(state) {
    if (!ZUNIGA_SCENARIOS.some(scenario => scenario.id === state?.scenarioId)) throw new Error('The saved Zuñiga scenario is invalid.');
    if (!['absolute', 'difference'].includes(state.comparison ?? 'absolute')) throw new Error('The saved Zuñiga comparison is invalid.');
    if (state.fluxReference !== undefined && state.fluxReference !== this.zunigaState().fluxReference) throw new Error('The saved Zuñiga map uses a different flux scale reference.');
    if (state.sourceHashes && Object.entries(this.zunigaStudy.sourceHashes).some(([file, hash]) => state.sourceHashes[file] !== hash)) throw new Error('The saved Zuñiga document refers to different source model hashes.');
  }

  async importZunigaBundle(bundle, check) {
    const archived = bundle.study;
    if (!archived.sourceModels?.mono || !archived.sourceModels?.joint) throw new Error('The private Zuñiga bundle lacks its complete source models.');
    const study = {
      manifest: archived.manifest,
      sourceHashes: archived.sourceHashes,
      sourceModels: archived.sourceModels,
      models: Object.fromEntries(['Ec', 'Se'].map(id => [id, new Model(bundle.models[id])])),
    };
    await this.useZunigaStudy({ study, scenarioId: archived.scenarioId });
    this.setZunigaComparison(archived.comparison ?? 'absolute');
    if (bundle.map) {
      const context = createZunigaScenario(study, 'mutant-coculture').community;
      validateMapDocument(bundle.map, context, this.species);
      this.map = EditableMap.fromJSON(bundle.map);
      this.mapHistory.clear();
    }
    const replay = compareReplayedSolution(bundle.solution, this.solution, bundle.replay?.tolerances ?? { absolute: 1e-7, relative: 1e-6 });
    check.replay = replay;
    this.reproducibilityReplay = { integrity: check, replay, archivedAt: bundle.metadata?.createdAt ?? null, restored: { map: Boolean(bundle.map), omicsExpected: false, omics: false } };
    this.settleOpenedWork();
    this.status = { ...this.status, state: replay.pass ? 'ok' : 'warn', message: `Private Zuñiga bundle restored · ${replay.pass ? 'numerical replay OK' : 'numerical differences'} · SHA-256 ${bundle.integrity.payloadHash.slice(0, 12)}…` };
    this.emit('rebuild');
    return check;
  }

  /**
   * Load a published community as the active one.
   *
   * The biological model and the initial map are deliberately separate.
   * Published communities that bundle full member models use those models for
   * editing, reaction discovery and simulation. A curated preview may still be
   * used to decide what is drawn initially; it must never truncate Edit Model.
   */
  async usePublishedCommunity(id) {
    if (id === ZUNIGA_ID) return this.useZunigaStudy();
    this.resetZunigaStudy();
    this.invalidateSteadyCom();
    this.steadyComConfig = { fiber: null, parsimonious: false };
    const community = this.publishedCommunities.find((c) => c.id === id);
    if (!community) return this;

    this.publishedCommunityId = id;
    this.steadyComState = null;
    this._steadyComSpec = null;
    // Selecting a community is first of all a request to read about it, so the
    // selection lands before anything is fetched. One whose sources could not be
    // retrieved has no models to draw, and that must not stop its paper, its
    // provenance and what it would still need from being shown.
    this.scenario = {
      id: `published:${id}`,
      name: community.name,
      description: community.description,
      published: id,
    };

    const manifest = await this.publishedManifest(id);
    if (!manifest?.members?.length) {
      // Do not leave the previous community/map on screen under the newly
      // selected scenario name. A published entry without bundled models has no
      // biological map to show, so clear the old active model explicitly.
      this.community = null;
      this.problem = null;
      this.map = null;
      this.solution = null;
      this.scale = null;
      this.crossFeeding = { links: [], perMetabolite: [] };
      this.growth = new Map();
      this.mapHistory.clear();
      this.selectedReaction = null;
      this.selectedGeometry = null;
      this.selectedEntity = null;
      this.multiSelection = emptySelection();
      this.variability = null;
      this.status = {
        ...this.status,
        state: 'warn',
        message: `${community.name}: no model files are bundled, so there is no map to draw.`,
      };
      this.settleOpenedWork();
      this.emit('rebuild');
      return this;
    }
    this.objective = OBJECTIVES.EQUAL_GROWTH;
    this.productTarget = null;
    this.parsimonious = false;
    this.pfbaBiomassPercent = 100;
    this.loopless = false;
    this.fvaEnabled = false;
    this.fvaRanges = new Map();
    this.fvaProgress = { running: false, done: 0, total: 0, ms: 0 };
    this.medium = { ...(manifest.medium ?? {}) };
    this.overrides.clear();
    this.manualPathways.clear();
    this.pathwaySettings.clear();
    this.clearOmics({ emit: false });
    this.selectedReaction = null;
    this.selectedEntity = null;
    this.variability = null;
    this.species = manifest.members.map((m) => {
      const biologicalFile = publishedMemberFileForRuntime(id, m);
      return {
        id: m.id,
        file: `published/${id}/${biologicalFile.replace(/\.json$/, '')}`,
        abundance: 1 / manifest.members.length,
        enabled: true,
      };
    });
    this.modelEditorSpeciesId = this.species[0]?.id ?? null;
    this.modelEditorMode = null;
    if (id === 'steadycom-ecoli-auxotrophs') {
      const members = [];
      for (const sp of this.species) members.push({ id: sp.id, model: await this.loadModel(sp.file) });
      this.medium = parseMediaCSV(await fetchText(`${this.base}/models/published/${id}/media.csv`), members);
    } else if (id === 'steadycom-gut-nine') {
      const members = [];
      for (const sp of this.species) members.push({ id: sp.id, model: await this.loadModel(sp.file) });
      const joint = jointSteadyComSpec(members, await fetchText(`${this.base}/models/published/${id}/bounds.csv`));
      this.medium = Object.fromEntries([...joint.metadata.communityUptake].map(([mid, rid]) =>
        [joint.metadata.bases.get(mid), joint.reactions.get(rid).upperBound]).filter(([, value]) => value > 0));
    }
    // The four-E. coli benchmark has genome-scale member models plus a curated
    // central-metabolism map. Start from an empty drawing shell and populate only
    // that readable overview; the active biological model remains full iAF1260.
    const curatedCoreMap = usesCuratedCoreMap(id);
    const jointMap = id === 'steadycom-gut-nine';
    await this.rebuild({ relayout: true, contents: curatedCoreMap || jointMap ? 'none' : 'all', solve: !jointMap });
    if (curatedCoreMap) {
      this.showCoreMetabolismOverview({ recordHistory: false });
    }
    this.settleOpenedWork();
    this.drawWaitingMap();
    return this;
  }

  /** Draw a small excerpt while keeping full models available for analysis. */
  async drawPublishedPreview(id) {
    const manifest = await this.publishedManifest(id);
    // A curated map, when the community ships one, replaces the excerpt. It is
    // read the way a loaded map file is, so it is checked against the full models.
    if (manifest?.map) {
      const drawing = await fetchJSON(`${this.base}/models/published/${id}/${manifest.map}`);
      this.mapLoadNote = '';
      this.applyMapDocument(parseMapDocument(drawing));
      this.mapHistory.clear();
      this.status = { ...this.status, state: 'ok', message: 'Full nine-species model loaded with its curated map. Calculate the permitted μ range to run SteadyCom.' };
      this.emit('map');
      return;
    }
    const entries = [];
    for (const member of manifest?.members ?? []) {
      const sp = this.species.find((entry) => entry.id === member.id);
      if (!sp || sp.enabled === false) continue;
      entries.push({ id: member.id, model: await this.loadModel(`published/${id}/${member.preview.replace(/\.json$/, '')}`), abundance: sp.abundance, enabled: true });
    }
    const drawing = new CommunityModel({ name: this.scenario.name, objective: this.objective, species: entries, medium: this.mediumSpec() });
    this.map = EditableMap.fromCommunity(drawing, layoutEngineOptions(this.layoutSettings));
    this.mapHistory.clear();
    this.status = { ...this.status, state: 'ok', message: 'Full nine-species model loaded; the map shows an excerpt. Calculate the permitted μ range to run SteadyCom.' };
    this.emit('map');
  }

  /** Rebuild the merged model and the LP; optionally redo the geometry. */
  async rebuild({ relayout = true, contents = 'all', solve = true } = {}) {
    if (this.isZunigaStudy) {
      if (solve) this.solve();
      return this;
    }
    this.invalidateSteadyCom();
    this.status = { ...this.status, state: 'busy', message: 'Building model…' };
    this.emit('status');
    await frame();

    const entries = [];
    for (const sp of this.species) {
      const model = await this.loadModel(sp.file);
      entries.push({ id: sp.id, model, abundance: sp.abundance, enabled: sp.enabled });
    }

    if (!entries.some((e) => e.enabled)) {
      this.status = { state: 'error', message: 'Enable at least one species.', solveMs: 0, layoutMs: 0 };
      this.community = null;
      this.problem = null;
      this.map = null;
      this.solution = null;
      this.emit('rebuild');
      return;
    }

    this.community = new CommunityModel({
      name: this.scenario?.name ?? 'Community',
      objective: this.objective,
      productTarget: this.productTarget,
      species: entries,
      medium: this.mediumSpec(),
    });

    this._jointProjectionSpec = null;
    if (this.scenario?.published === 'steadycom-gut-nine') {
      const spec = jointSteadyComSpec(entries.map(({ id, model }) => ({ id, model })),
        await fetchText(`${this.base}/models/published/steadycom-gut-nine/bounds.csv`));
      this._jointProjectionSpec = spec;
      this.installJointProjection();
    }
    this.problem = new FluxProblem(this.community.merged);
    this.defaultBounds = this.problem.snapshotBounds();
    this.applyOverrides();

    if (relayout || !this.map) {
      const t = performance.now();
      // The automatic layout still produces the geometry; the map adopts it and
      // owns it from here on, which is what makes it editable.
      const layoutOptions = layoutEngineOptions(this.layoutSettings);
      this.map = contents === 'none' || this.scenario?.published === 'steadycom-gut-nine'
        ? EditableMap.emptyFromCommunity(this.community, layoutOptions)
        : EditableMap.fromCommunity(this.community, layoutOptions);
      this.mapHistory.clear();
      this.status.layoutMs = Math.round(performance.now() - t);
    }

    if (solve) {
      this.solve({ silent: true });
    } else {
      this.solution = null;
      this.refreshFluxScale();
      this.crossFeeding = { links: [], perMetabolite: [] };
      this.crossFeedingExplorer.index = null;
      this.crossFeedingExplorer.selectedId = null;
      this.growth = new Map();
      this.status = {
        ...this.status,
        state: 'ok',
        message: 'Model loaded. Analysis is ready to run on demand.',
        solveMs: 0,
      };
    }
    if (relayout && this.scenario?.published === 'steadycom-gut-nine') await this.drawPublishedPreview(this.scenario.published);
    this.emit('rebuild');
  }

  /** Medium limits in the shape CommunityModel expects. */
  mediumSpec() {
    const out = {};
    for (const [base, limit] of Object.entries(this.medium)) {
      out[base] = { lower: -Math.abs(Number(limit) || 0), upper: 1000 };
    }
    return out;
  }

  /** Reset the LP to its default bounds, then apply the medium and the per-reaction overrides. */
  applyOverrides() {
    if (!this.problem) return;
    this.problem.restoreBounds(this.defaultBounds);
    if (this.isZunigaStudy) return;
    for (const [base, limit] of Object.entries(this.medium)) {
      const uptakeId = this.community?.communityExchangeIds?.get(base)?.uptake;
      if (uptakeId && this.problem.rxnIndex.has(uptakeId)) {
        const index = this.problem.rxnIndex.get(uptakeId);
        this.problem.setBounds(uptakeId, this.defaultBounds.lb[index], Math.abs(Number(limit) || 0));
        continue;
      }
      const exId = communityExchange(base);
      if (this.problem.rxnIndex.has(exId)) {
        this.problem.setBounds(exId, -Math.abs(Number(limit) || 0), 1000);
      }
    }
    // Per-reaction overrides win over the medium, so a knockout stays a knockout.
    // A held-out species keeps its edited bounds for when it returns, but they
    // must not reopen what holding it out closed: in a joint community its
    // reactions stay in the problem, held at zero.
    const off = new Set(this.species.filter((sp) => this.speciesHeldOut(sp.id)).map((sp) => sp.id));
    for (const [rid, b] of this.overrides) {
      if (!this.problem.rxnIndex.has(rid)) continue;
      if (off.size && off.has(this.community?.reactionOwner?.get(rid)?.speciesId)) continue;
      this.problem.setBounds(rid, b.lower, b.upper);
    }
  }

  /**
   * Whether a species is out of the simulation: switched off, or, in a joint
   * community, given zero abundance, which closes its reactions the same way.
   * Its bounds can be neither applied nor edited until it is back.
   */
  speciesHeldOut(id) {
    const sp = this.species.find((item) => item.id === id);
    if (!sp) return false;
    if (sp.enabled === false) return true;
    return Boolean(this._jointProjectionSpec) && !((this.community?.abundances?.get(id) ?? 0) > 1e-9);
  }

  /** Replace the merged model with the joint SteadyCom projection at the current abundances. */
  installJointProjection() {
    const amounts = new Map(this._jointProjectionSpec.members.map(member =>
      [member.id, this.community.abundances.get(member.id) ?? 0]));
    const projection = jointCommunityProjection(this._jointProjectionSpec, amounts, { objective: this.objective, productTarget: this.productTarget });
    this.community.merged = projection.model;
    for (const name of ['biomassIds', 'reactionOwner', 'metaboliteOwner', 'transfersByBase', 'speciesCompartments', 'communityExchangeIds', 'abundances']) {
      this.community[name] = projection[name];
    }
  }

  /** Rebuild the LP for the current abundances and switched-off species, keeping the map. */
  refreshCompositionProblem() {
    for (const member of this.community.species) {
      const species = this.species.find(sp => sp.id === member.id);
      member.abundance = species?.abundance ?? 0;
      member.enabled = species?.enabled !== false && member.abundance > 1e-9;
    }
    this.community.abundances = new Map(this.species.map(sp => [sp.id, sp.enabled === false ? 0 : sp.abundance]));
    if (this._jointProjectionSpec) this.installJointProjection();
    else this.community.build();
    this.problem = new FluxProblem(this.community.merged);
    this.defaultBounds = this.problem.snapshotBounds();
    this.applyOverrides();
  }

  /** Install a solver result and refresh every state derived from it. */
  applySolution(res, { allowFVA = true } = {}) {
    this.solution = res;
    this.refreshFluxScale();
    if (res?.status === 'optimal') {
      this.crossFeeding = res.steadyCom ? steadyComCrossFeeding(res.steadyCom.spec, res.steadyCom.originalFluxes)
        : this._jointProjectionSpec ? steadyComCrossFeeding(this._jointProjectionSpec, new Map([...res.fluxes].map(([id, value]) => {
          const { species, id: original } = splitId(id);
          return [original, value * (species ? this.community.abundances.get(species) ?? 0 : 1)];
        }))) : this.community.crossFeeding(res.fluxes, 1e-5);
      this.growth = this.community.growthRates(res.fluxes);
      this.refreshCrossFeedingExplorer();
      if (allowFVA && this.fvaEnabled) this.scheduleFVA();
      else if (!allowFVA && this.fvaEnabled) {
        this.fvaEnabled = false;
        this.cancelFVA({ clear:true, emit:false });
      }
    } else {
      this.crossFeeding = { links:[], perMetabolite:[] };
      this.growth = new Map();
      this.crossFeedingExplorer.index = null;
      this.crossFeedingExplorer.selectedId = null;
      this.cancelFVA({ clear:true, emit:false });
    }
    return res;
  }

  /** Solve and refresh everything derived from the solution. */
  solve({ parsimonious = null, silent = false } = {}) {
    if (!this.problem) return null;
    if (this.isZunigaStudy) {
      const t = performance.now();
      const result = solveZunigaScenario(this.zunigaPrepared);
      result.zuniga.growth = Object.fromEntries(this.community.growthRates(result.fluxes));
      result.zuniga.crossFeeding = this.community.crossFeeding(result.fluxes);
      this.zunigaSolutions.set(this.zunigaScenarioId, result);
      this.applySolution(result, { allowFVA: false });
      // Like an ordinary solve, a success leaves the header to its summary and
      // goes to the log. Shown as a notice it would hide the objective for six
      // seconds after every re-solve.
      const caseName = `${ZUNIGA_SCENARIOS.find(s => s.id === this.zunigaScenarioId)?.name} · exact source constraints`;
      const optimal = result.status === 'optimal';
      this.status = { ...this.status, state: optimal ? 'ok' : 'error', message: optimal ? '' : caseName, solveMs: Math.round(performance.now() - t) };
      if (optimal) this.addLog('ok', `${caseName} solved: optimal, ${this.status.solveMs} ms`, { kind: 'solve' });
      if (!silent) this.emit('solution');
      return result;
    }
    this.cancelSteadyCom({ silent: true });
    if (this.solution?.steadyCom) this.refreshCompositionProblem();
    const useParsimonious = parsimonious === null ? this.parsimonious : Boolean(parsimonious);
    const t = performance.now();
    const res = this.loopless
      ? this.problem.loopless()
      : useParsimonious
        ? this.problem.parsimonious({ fraction: this.pfbaBiomassPercent / 100 })
        : this.problem.optimize();
    this.applySolution(res, { allowFVA:true });
    this.status = {
      state: res.status === 'optimal' ? 'ok' : 'error',
      message: res.status === 'optimal' ? '' : (res.message || describeStatus(res.status)),
      solveMs: Math.round(performance.now() - t),
      layoutMs: this.status.layoutMs ?? 0,
    };
    if (res.status === 'optimal') {
      const method = this.loopless ? 'Loopless FBA' : (useParsimonious ? 'pFBA' : 'FBA');
      const objective = Number.isFinite(this.solution?.objectiveValue) ? this.solution.objectiveValue.toFixed(5) : '—';
      this.addLog('ok', `${method} solved: optimal, objective ${objective}, ${this.status.solveMs} ms`, { kind: 'solve' });
    }
    if (!silent) this.emit('solution');
    return res;
  }

  /**
   * Run one of the analyses of the Simulate panel, as its buttons do, and say
   * when it has finished.
   *
   * The solves are synchronous. On a genome-scale community a pFBA or loopless
   * solve holds the page for seconds and may change nothing anyone can see, so
   * without a word there is no telling whether it has finished, or started. The
   * run is announced first, with a paint in between, and its end is kept in
   * `lastRun` for the results bar, and goes to the status line and the log.
   * FVA has finished when its ranges are in, not when it is started.
   *
   * @param {'fba'|'pfba'|'fva'|'loopless'} method
   */
  async runAnalysis(method) {
    const label = ANALYSIS_LABELS[method];
    if (!label) throw new Error(`Unknown analysis "${method}".`);
    if (this.isZunigaStudy && method !== 'fba') return this.solution;
    if (method === 'fva' && this.solution?.steadyCom) return this.solution;
    this.lastRun = { method, label, phase: 'running', startedAt: Date.now(), t0: performance.now() };
    this.status = { ...this.status, state: 'busy', message: `Running ${label}…` };
    this.emit('status');
    await paintFrame();
    if (this.lastRun?.method !== method || this.lastRun.phase !== 'running') return this.solution;

    if (this.isZunigaStudy) {
      this.solve();
    } else if (method === 'fba') {
      this.parsimonious = false;
      this.loopless = false;
      this.setFVA(false);
      this.solve();
    } else if (method === 'pfba') {
      this.loopless = false;
      this.setFVA(false);
      this.setParsimonious(true);
    } else if (method === 'loopless') {
      this.setLoopless(true);
    } else {
      this.loopless = false;
      this.parsimonious = false;
      this.solve();
      this.setFVA(true);
      // Its ranges come in later; runFVA() finishes the run then.
      if (this.solution?.status === 'optimal' && this.fvaProgress?.running) {
        this.status = { ...this.status, state: 'busy', message: 'Running FVA…' };
        this.emit('status');
        return this.solution;
      }
    }
    this.finishRun(this.problem ? (this.solution?.status ?? 'not run') : 'no model loaded');
    return this.solution;
  }

  /** Close the run `runAnalysis` started, with the outcome it ended in. */
  finishRun(outcome) {
    const run = this.lastRun;
    if (!run || run.phase !== 'running') return;
    const ms = Math.round(performance.now() - run.t0);
    const ok = outcome === 'optimal';
    const objective = this.solution?.objectiveValue;
    Object.assign(run, {
      phase: ok ? 'done' : outcome === 'cancelled' ? 'cancelled' : 'failed',
      outcome, ms, took: formatDuration(ms), finishedAt: Date.now(), objective,
    });
    const took = formatDuration(ms);
    if (ok) {
      const detail = run.method === 'fva'
        ? `${this.fvaRanges.size} flux ranges`
        : `objective ${Number.isFinite(objective) ? objective.toFixed(5) : '—'}`;
      this.status = { ...this.status, state: 'ok', message: `${run.label} completed · ${detail} · ${took}` };
    } else if (outcome === 'cancelled') {
      this.status = { ...this.status, state: 'ok', message: `${run.label} cancelled after ${took}` };
    } else {
      const why = !this.problem ? 'no model is loaded yet.'
        : (this.status?.state === 'error' && this.status.message) || describeStatus(outcome);
      this.status = { ...this.status, state: 'error', message: `${run.label} did not finish: ${why}` };
    }
    this.emit('status');
  }

  // Mutations

  setParsimonious(enabled) {
    this.assertPublishedStudyConstraints();
    const next = Boolean(enabled);
    if (next && this.loopless) this.loopless = false;
    if (this.parsimonious === next && this.solution) return this.solution;
    this.parsimonious = next;
    return this.solve();
  }

  setPFBABiomassPercent(value) {
    this.assertPublishedStudyConstraints();
    if (String(value ?? '').trim() === '') return this.solution;
    const next = normalizePfbaPercent(value);
    if (Math.abs(next - this.pfbaBiomassPercent) < 1e-12) return this.solution;
    this.pfbaBiomassPercent = next;
    if (this.parsimonious && !this.loopless) return this.solve();
    this.emit('pfba-fraction');
    return this.solution;
  }

  setLoopless(enabled) {
    this.assertPublishedStudyConstraints();
    const next = Boolean(enabled);
    if (this.loopless === next && this.solution) return this.solution;
    this.loopless = next;

    if (next) {
      this.parsimonious = false;
      if (this.fvaEnabled) this.setFVA(false);
    }

    return this.solve();
  }

  /** Switch FVA on (ranges computed in the background after each solve) or off. */
  setFVA(enabled) {
    this.assertPublishedStudyConstraints();
    if (enabled && this.solution?.steadyCom) {
      this.status = { ...this.status, state: 'warn', message: 'Ordinary FVA does not apply to the fixed-μ SteadyCom LP. Run an ordinary analysis first.' };
      this.emit('status');
      return this.fvaRanges;
    }
    const next = Boolean(enabled);
    if (next && this.loopless) {
      this.loopless = false;
      this.solve();
    }
    if (this.fvaEnabled === next) return this.fvaRanges;
    this.fvaEnabled = next;

    if (!next) {
      this.cancelFVA({ clear: true, emit: false });
      this.variability = null;
      this.emit('fva');
      if (this.lastRun?.method === 'fva') this.finishRun('cancelled');
      return this.fvaRanges;
    }

    this.scheduleFVA(0);
    this.emit('fva');
    return this.fvaRanges;
  }

  cancelFVA({ clear = false, emit = true } = {}) {
    this._fvaGeneration += 1;
    if (this._fvaTimer !== null) {
      clearTimeout(this._fvaTimer);
      this._fvaTimer = null;
    }
    this.fvaProgress = { running: false, done: 0, total: 0, ms: 0 };
    if (clear) {
      this.fvaRanges = new Map();
      this.variability = null;
    }
    if (emit) this.emit('fva');
  }

  /** Start a fresh FVA run after `delay` ms, cancelling any run in progress. */
  scheduleFVA(delay = 40) {
    if (!this.fvaEnabled || !this.problem || this.solution?.status !== 'optimal') return;
    const generation = ++this._fvaGeneration;
    if (this._fvaTimer !== null) clearTimeout(this._fvaTimer);

    const total = this.problem.rxnIds.length;
    this.fvaRanges = new Map();
    this.variability = null;
    this.fvaProgress = { running: true, done: 0, total, ms: 0 };

    this._fvaTimer = setTimeout(() => {
      this._fvaTimer = null;
      this.runFVA(generation);
    }, Math.max(0, Number(delay) || 0));
  }

  /** Compute every reaction's flux range in chunks, publishing partial ranges, until a newer run supersedes it. */
  async runFVA(generation = this._fvaGeneration) {
    if (!this.fvaEnabled || !this.problem || generation !== this._fvaGeneration) return new Map();

    const problem = this.problem;
    const ids = [...problem.rxnIds];
    const t = performance.now();

    const ranges = await problem.variabilityAsync(ids, {
      fraction: 1.0,
      chunkSize: 20,
      shouldContinue: () =>
        this.fvaEnabled
        && generation === this._fvaGeneration
        && this.problem === problem,
      onProgress: (partial, done, total) => {
        if (generation !== this._fvaGeneration || this.problem !== problem) return;
        this.fvaRanges = new Map(partial);
        this.variability = this.selectedReaction
          ? this.fvaRanges.get(this.selectedReaction) ?? null
          : null;
        this.fvaProgress = {
          running: done < total,
          done,
          total,
          ms: Math.round(performance.now() - t),
        };
        this.emit('fva-progress');
      },
    });

    if (!this.fvaEnabled || generation !== this._fvaGeneration || this.problem !== problem) return ranges;

    this.fvaRanges = new Map(ranges);
    this.variability = this.selectedReaction
      ? this.fvaRanges.get(this.selectedReaction) ?? null
      : null;
    this.fvaProgress = {
      running: false,
      done: this.fvaRanges.size,
      total: ids.length,
      ms: Math.round(performance.now() - t),
    };
    this.emit('fva');
    if (this.lastRun?.method === 'fva') this.finishRun('optimal');
    return this.fvaRanges;
  }

  // SteadyCom

  /*
   * SteadyCom asks a different question from the one the map answers by default.
   *
   * The map runs joint FBA with one growth rate shared by every member, picked by
   * the LP. SteadyCom instead treats the growth rate as the thing being searched
   * for: fix mu and the problem becomes an ordinary LP in the fluxes and the
   * abundances, and the community can hold together at any mu up to the largest
   * one that still supports the required biomass. So there is a whole range of
   * admissible growth rates, each with its own flux distribution and its own
   * population composition, and only the top of that range is a single number.
   *
   * This exposes both: the range, and the solve at a growth rate the user picks
   * inside it.
   */

  /**
   * Whether a community can be posed as a SteadyCom problem here.
   *
   * Takes an id because the card in the Scenario workspace shows whichever
   * community is *selected* in the list, which is not always the one that is
   * open. Asked about the selected one, the answer is about that one.
   */
  steadyComSupport(communityId = this.scenario?.published ?? this.scenario?.id) {
    const active = this.scenario?.published ?? this.scenario?.id;
    if (!communityId || communityId !== active) {
      return { supported: false, reason: 'Open this community first to load its models.' };
    }
    if (this.scenario?.published && !STEADYCOM_COMMUNITIES.has(communityId)) {
      return { supported: false, reason: 'This published example does not supply a validated SteadyCom formulation and its full constraints.' };
    }
    if (!this.community || !this.species.some((sp) => sp.enabled !== false)) {
      return { supported: false, reason: 'No active member models are available.' };
    }
    const embedded = globalThis.__ESCHER_MCMC_DATA__;
    const manifest = this.publishedManifests.get(communityId);
    if (embedded && this.scenario?.published && !(manifest?.members ?? []).every((member) =>
      `models/published/${communityId}/${member.model}` in embedded)) {
      return { supported: false, reason: 'This older standalone contains only map excerpts. Open the updated standalone or serve the application to calculate growth rates.' };
    }
    return { supported: true, communityId, kind: communityId === 'steadycom-gut-nine' ? 'joint' : 'separate', reason: '' };
  }

  /** What a computed SteadyCom range depends on, as a string: a range under another key is stale. */
  steadyComKey() {
    return JSON.stringify({
      community: this.scenario?.published ?? this.scenario?.id,
      revision: this._steadyComRevision,
      members: this.species.map((sp) => [sp.id, sp.file, sp.enabled !== false]),
      medium: Object.entries(this.medium).sort(([a], [b]) => a.localeCompare(b)),
      overrides: [...this.overrides].sort(([a], [b]) => a.localeCompare(b)),
      fiber: this.steadyComConfig.fiber,
    });
  }

  cancelSteadyCom({ silent = false } = {}) {
    const running = Boolean(this._steadyComController);
    this._steadyComGeneration += 1;
    this._steadyComController?.abort();
    this._steadyComController = null;
    if (this.steadyComState) this.steadyComState.phase = null;
    if (running && !silent) {
      this.status = { ...this.status, state: 'ok', message: 'SteadyCom calculation cancelled.' };
      this.emit('steadycom');
    }
  }

  /** Forget the SteadyCom range and problem, and any fixed-µ solution on screen. */
  invalidateSteadyCom() {
    this.cancelSteadyCom({ silent: true });
    this._steadyComRevision += 1;
    this.steadyComState = null;
    this._steadyComSpec = null;
    if (this.solution?.steadyCom) {
      this.solution = null;
      this.refreshFluxScale();
      this.crossFeeding = { links: [], perMetabolite: [] };
      this.growth = new Map();
      this.crossFeedingExplorer.index = null;
    }
  }

  /** The B. thetaiotaomicron fiber limit of the nine-species example (C-mmol gDW⁻¹ h⁻¹, 1 to 30), or null for the default. */
  setSteadyComFiber(value) {
    if (this.scenario?.published !== 'steadycom-gut-nine') throw new Error('Fiber conditions apply to the nine-species example.');
    const rate = value === null ? null : Number(value);
    if (rate !== null && (!Number.isFinite(rate) || rate < 1 || rate > 30)) {
      throw new Error('Choose a B. thetaiotaomicron fiber limit between 1 and 30 C-mmol gDW⁻¹ h⁻¹.');
    }
    this.invalidateSteadyCom();
    this.steadyComConfig.fiber = rate === null ? null : { bacteroides: rate, faecalibacterium: 5 };
    this.status = { ...this.status, state: 'ok', message: 'Condition changed. Calculate the permitted growth-rate range again.' };
    this.emit('steadycom');
  }

  /**
   * Whether a fixed-µ solve returns the cheapest flux distribution (least total
   * member flux) or the first feasible one. It does not change the µ range, so
   * the range stays valid; only the next solve reads it.
   */
  setSteadyComParsimonious(on) {
    this.steadyComConfig.parsimonious = Boolean(on);
    if (this.solution?.steadyCom && Boolean(this.solution.steadyCom.parsimonious) !== this.steadyComConfig.parsimonious) {
      this.status = { ...this.status, state: 'ok', message: 'Solve at this µ again to apply the change.' };
    }
    this.emit('steadycom');
  }

  /** The SteadyCom problem for the open community with the current medium and overrides, cached under steadyComKey. */
  async steadyComProblem({ recompute = false } = {}) {
    const support = this.steadyComSupport();
    if (!support.supported) throw new Error(support.reason);
    const key = this.steadyComKey();
    if (!recompute && this._steadyComSpec?.key === key) return this._steadyComSpec.spec;
    const members = [];
    for (const sp of this.species) {
      if (sp.enabled === false && support.kind !== 'joint') continue;
      const model = await this.loadModel(sp.file);
      members.push({ id: sp.id, model: new Model(model.toJSON()) });
    }
    let spec;
    if (support.kind === 'joint') {
      const bounds = await fetchText(`${this.base}/models/published/${support.communityId}/bounds.csv`);
      spec = jointSteadyComSpec(members, bounds, { fiber: this.steadyComConfig.fiber });
      for (const sp of this.species.filter(sp => sp.enabled === false)) {
        spec.constraints ??= [];
        spec.constraints.push({ name: `disabled|${sp.id}`, biomass: [{ species: sp.id, coef: 1 }], lo: 0, hi: 0 });
      }
      for (const [mid, rid] of spec.metadata.communityUptake) {
        const base = spec.metadata.bases.get(mid);
        if (base in this.medium) spec.reactions.get(rid).upperBound = this.medium[base];
      }
      for (const [id, bounds] of this.overrides) {
        const { id: rid, species } = splitId(id);
        const reaction = spec.reactions.get(rid);
        if (reaction && ((!species && reaction.species === 'comm') || reaction.species === species)) {
          reaction.lowerBound = bounds.lower; reaction.upperBound = bounds.upper;
        }
      }
    } else {
      spec = steadyComSpec(members, this.medium, 1);
      for (const [id, bounds] of this.overrides) {
        const { id: rid, species } = splitId(id);
        if (species) {
          const reaction = members.find((member) => member.id === species)?.model.reactions.get(rid);
          if (reaction) { reaction.lowerBound = bounds.lower; reaction.upperBound = bounds.upper; }
        } else {
          const bases = new Set(spec.members.flatMap(member => Object.keys(member.exchangeFor)));
          for (const base of bases) {
            if (communityExchange(base) !== id) continue;
            spec.communityExchangeBounds ??= {};
            spec.communityExchangeBounds[base] = { lower: bounds.lower, upper: bounds.upper };
            spec.communityUptake[base] = Math.max(0, -bounds.lower);
          }
        }
      }
    }
    if (this.steadyComKey() !== key) throw staleSteadyComError();
    this._steadyComSpec = { key, communityId: support.communityId, spec };
    return spec;
  }

  /** Search the range of admissible community growth rates (h⁻¹). Resolves to the SteadyCom state with `min` and `max`. */
  async steadyComRange({ recompute = false } = {}) {
    const key = this.steadyComKey();
    if (!recompute && this.steadyComState?.key === key && Number.isFinite(this.steadyComState.max)) return this.steadyComState;
    this.cancelSteadyCom({ silent: true });
    const generation = this._steadyComGeneration;
    const controller = new AbortController();
    this._steadyComController = controller;
    const id = this.scenario?.published ?? this.scenario?.id;
    const state = { communityId: id, key, min: null, max: null, source: 'computed', phase: 'range', progress: null, mu: null, feasible: null, abundances: new Map() };
    this.steadyComState = state;
    this.status = { ...this.status, state: 'busy', message: 'Calculating permitted community growth rates…' };
    this.emit('steadycom');
    const started = performance.now();
    try {
      const spec = await this.steadyComProblem({ recompute });
      // Near the top of the range the nine-species LPs get slow: with the Figure 4
      // fiber limit (Bt 4) the step at 0.0575 h⁻¹ takes more than 30 s, which
      // would fail the whole search. Those steps get more time, and the task as a
      // whole too.
      const nine = id === 'steadycom-gut-nine';
      const result = await this.runSteadyComTask(spec, {
        type: 'range', options: { upperGuess: nine ? 0.1 : 1, tolerance: 1e-5,
          biomassCap: steadyComCapIsSafe(spec) ? 2 * (spec.totalBiomass ?? 1) : null, timeLimitSeconds: nine ? 180 : 30 },
      }, { signal: controller.signal, ...(nine ? { timeoutMs: 1800000 } : {}), onProgress: (progress) => {
        if (generation !== this._steadyComGeneration || this.steadyComKey() !== key) return;
        state.progress = progress;
        this.status.message = `Calculating μ range: ${progress.iterations} LP evaluations…`;
        this.emit('steadycom');
      } });
      if (generation !== this._steadyComGeneration || this.steadyComKey() !== key) throw staleSteadyComError();
      if (result.status !== 'optimal' || !result.zeroFeasible || !Number.isFinite(result.rangeUpper)) {
        throw new Error(result.message || 'The solver did not establish a permitted growth-rate interval.');
      }
      Object.assign(state, { min: result.rangeLower, max: result.rangeUpper, upperBound: result.upperBound,
        resolution: result.resolution, iterations: result.iterations, seconds: Math.round((performance.now() - started) / 100) / 10,
        phase: null, progress: null, error: null });
      this.status = { ...this.status, state: 'ok', message: `Permitted μ: ${state.min}–${state.max.toFixed(6)} h⁻¹ for the current constraints.` };
      this.emit('steadycom');
      return state;
    } catch (error) {
      if (generation === this._steadyComGeneration) {
        state.phase = null; state.error = error.message;
        this.status = { ...this.status, state: error.name === 'AbortError' ? 'ok' : 'error', message: error.message };
        this.emit('steadycom');
      }
      throw error;
    } finally {
      if (this._steadyComController === controller) this._steadyComController = null;
    }
  }

  /**
   * Solve SteadyCom at a fixed growth rate `mu` (h⁻¹) inside the computed range,
   * and install the resulting abundances and fluxes as the solution. Fluxes are
   * per unit biomass of each member.
   */
  async solveSteadyComAt(mu) {
    if (typeof mu !== 'number' || !Number.isFinite(mu) || mu < 0) throw new Error('Enter a finite growth rate of zero or more.');
    const key = this.steadyComKey();
    const state = this.steadyComState?.key === key && Number.isFinite(this.steadyComState.max)
      ? this.steadyComState : await this.steadyComRange();
    if (mu < state.min || mu > state.max) throw new Error(`Choose μ inside the computed range ${state.min}–${state.max} h⁻¹.`);
    this.cancelSteadyCom({ silent: true });
    const generation = this._steadyComGeneration;
    const controller = new AbortController();
    this._steadyComController = controller;
    const parsimonious = Boolean(this.steadyComConfig?.parsimonious);
    state.phase = 'fixed'; state.parsimonious = parsimonious; state.error = null;
    this.status = { ...this.status, state: 'busy', message: `Calculating ${parsimonious ? 'the least total flux' : 'fluxes'} at fixed μ = ${mu} h⁻¹…` };
    this.emit('steadycom');
    const started = performance.now();
    try {
      const spec = await this.steadyComProblem();
      // The parsimonious LP carries an extra column and two rows per reversible
      // reaction and is an optimisation rather than a feasibility question, so
      // it gets more time than the plain solve.
      const at = parsimonious
        ? await this.runSteadyComTask(spec, { type: 'parsimonious', mu, options: { timeLimitSeconds: 600 } }, { signal: controller.signal, timeoutMs: 900000 })
        : await this.runSteadyComTask(spec, { type: 'fixed', mu, options: { timeLimitSeconds: 60 } }, { signal: controller.signal });
      if (generation !== this._steadyComGeneration || this.steadyComKey() !== key) throw staleSteadyComError();
      if (at.status !== 'optimal' || !at.feasible) {
        throw new Error(at.status === 'infeasible' ? `No feasible flux distribution was found at μ = ${mu} h⁻¹.`
          : `The solver did not finish the fixed-μ calculation (${at.status}). Its last solution has been retained.`);
      }
      const owners = spec.metadata?.reactionOwners ?? null;
      // An absent member's abundance is an LP column at its zero bound, which
      // can come back a hair below zero; anything further off is left to fail.
      const cleanAbundance = (x) => (x === undefined || x === null ? 0 : (x < 0 && x > -1e-9 ? 0 : x));
      const fluxes = new Map();
      for (const [id, value] of toMergedFluxes(at.fluxes, '@', owners)) {
        const { species } = splitId(id);
        const abundance = species ? cleanAbundance(at.abundances.get(species)) : 1;
        fluxes.set(id, abundance > 1e-9 ? value / abundance : 0);
      }
      for (const sp of this.species) sp.abundance = cleanAbundance(at.abundances.get(sp.id));
      this.community.abundances = new Map([...at.abundances].map(([id, x]) => [id, cleanAbundance(x)]));
      this.refreshCompositionProblem();
      this.parsimonious = false; this.loopless = false;
      this.omics.gimmeEnabled = false; this.omics.gimmeResult = null;
      this.setFVA(false);
      Object.assign(state, { phase: null, mu, feasible: true, abundances: at.abundances, totalBiomass: at.total,
        originalFluxes: at.fluxes, parsimonious, totalFlux: parsimonious ? at.totalFlux : null,
        solveMs: Math.round(performance.now() - started) });
      this.applySolution({ status: 'optimal', objectiveValue: mu, fluxes, solver: 'glpk',
        steadyCom: { mu, key, abundances: at.abundances, totalBiomass: at.total, spec, originalFluxes: at.fluxes,
          parsimonious, totalFlux: state.totalFlux } }, { allowFVA: false });
      this.status = { ...this.status, state: 'ok', message: parsimonious
        ? `SteadyCom at fixed μ = ${mu} h⁻¹, parsimonious: the composition and fluxes with the least total member flux (${at.totalFlux.toFixed(3)} mmol h⁻¹).`
        : `SteadyCom at fixed μ = ${mu} h⁻¹; one feasible population composition and flux distribution.`, solveMs: state.solveMs };
      this.emit('steadycom');
      return state;
    } catch (error) {
      if (generation === this._steadyComGeneration) {
        state.phase = null; state.error = error.message;
        this.status = { ...this.status, state: error.name === 'AbortError' ? 'ok' : 'error', message: error.message };
        this.emit('steadycom');
      }
      throw error;
    } finally {
      if (this._steadyComController === controller) this._steadyComController = null;
    }
  }

  /** Switch a species on or off and rebuild, keeping a hand-made drawing (see keepsDrawingOnSpeciesToggle). */
  async setSpeciesEnabled(id, enabled) {
    this.assertPublishedStudyConstraints();
    const sp = this.species.find((s) => s.id === id);
    if (!sp || sp.enabled === enabled) return;
    // With no species left there is no community, and the map would go with it.
    if (!enabled && !this.species.some((other) => other !== sp && other.enabled)) {
      this.status = { ...this.status, state: 'warn', message: 'At least one species has to stay enabled.' };
      this.emit('species');
      return;
    }
    const keepDrawing = this.keepsDrawingOnSpeciesToggle();
    sp.enabled = enabled;
    // Bounds edited on any species are kept. Those of a species that has left
    // are skipped while it is out (applyOverrides sets only what the problem
    // has) and apply again when it is back. Clearing them all would throw away
    // the edits on every species that stays.
    this.selectedReaction = null;
    this.selectedEntity = null;
    await this.rebuild({ relayout: !keepDrawing });
    if (keepDrawing && enabled && this.map && !this.map.cells.some((cell) => cell.id === id)) {
      this.status = {
        ...this.status,
        state: 'warn',
        message: `${id} is simulated again but is not drawn on this hand-built map. Draw its cell with the Cell tool, or use Draw full model.`,
      };
      this.emit('status');
    }
  }

  /**
   * Whether switching a species on or off keeps the map as drawn.
   *
   * A drawing made by hand is kept, with its undo history: a species that
   * leaves stays on it, dimmed and not simulated, and comes back as it was.
   * Only a map nobody has touched is laid out again. A genome-scale model never
   * is, since that would take minutes.
   */
  keepsDrawingOnSpeciesToggle() {
    // Edits waiting in Redo are hand-made too: laying the map out again would
    // clear the history and lose them.
    const handMade = this.mapHistory.canUndo || this.mapHistory.canRedo;
    return Boolean(this.map) && (handMade || this.anySpeciesTooLargeToAutoDraw());
  }

  /**
   * Like tooLargeToAutoDraw, but over every species of the community, switched
   * off ones included: switching a genome-scale species back on must not lay
   * the map out again either.
   */
  anySpeciesTooLargeToAutoDraw() {
    return this.species.some((sp) => {
      const model = this.models.get(sp.file);
      return Boolean(model) && (model.reactions.size > AUTO_DRAW_REACTION_LIMIT
        || model.metabolites.size > AUTO_DRAW_METABOLITE_LIMIT);
    });
  }

  /** Species that are switched off but still drawn on the map, dimmed. */
  disabledDrawnSpeciesIds() {
    const drawn = new Set((this.map?.cells ?? []).map((cell) => cell.id));
    return this.species.filter((sp) => !sp.enabled && drawn.has(sp.id)).map((sp) => sp.id);
  }

  async setAbundance(id, value) {
    this.assertPublishedStudyConstraints();
    const sp = this.species.find((s) => s.id === id);
    const abundance = finiteInput(value);
    if (!sp || abundance === null) return;
    sp.abundance = Math.max(0, abundance);
    // Abundance lives in the stoichiometry of the transfer reactions, so this is
    // structural; the geometry, however, does not depend on it.
    await this.rebuild({ relayout: false });
  }

  async setObjective(type, target = null) {
    this.assertPublishedStudyConstraints();
    this.objective = type;
    this.productTarget = target;
    await this.rebuild({ relayout: false });
  }

  /** Set the uptake limit of a medium metabolite (positive, mmol/gDW/h) and solve. */
  setMediumLimit(base, value) {
    this.assertPublishedStudyConstraints();
    // A cleared or half-typed field is not a limit of 0.
    const limit = finiteInput(value);
    if (limit === null) return null;
    this.invalidateSteadyCom();
    if (this.scenario?.published === 'steadycom-gut-nine') this.overrides.delete(`UT_${base}(u)`);
    this.medium[base] = Math.abs(limit);
    // Per-reaction overrides are applied after the medium, so an override left
    // on this exchange would swallow the slider and make it look broken. The
    // slider is the more direct expression of intent, so it wins.
    this.overrides.delete(communityExchange(base));
    this.applyOverrides();
    this.solve();
  }

  /**
   * Change a reaction's bounds without solving; caller decides when to solve.
   *
   * A side given as null, undefined, an empty string or anything that is not a
   * finite number is left as it is: a field the user has just cleared is not a
   * bound of 0.
   */
  setReactionBounds(rid, lower, upper) {
    this.assertPublishedStudyConstraints();
    const current = this.overrides.get(rid) || this.defaultReactionBounds(rid);
    const lowerValue = finiteInput(lower);
    const upperValue = finiteInput(upper);
    if (lowerValue === null && upperValue === null) return { ...current };
    this.invalidateSteadyCom();
    const next = {
      lower: lowerValue ?? current.lower,
      upper: upperValue ?? current.upper,
    };
    if (next.lower > next.upper) {
      if (lowerValue === null) next.upper = next.lower;
      else next.lower = next.upper;
    }
    this.overrides.set(rid, next);
    this.applyOverrides();
    return next;
  }

  defaultReactionBounds(rid) {
    const j = this.problem?.rxnIndex.get(rid);
    if (j === undefined) return { lower: 0, upper: 0 };
    return { lower: this.defaultBounds.lb[j], upper: this.defaultBounds.ub[j] };
  }

  currentReactionBounds(rid) {
    return this.overrides.get(rid) || this.defaultReactionBounds(rid);
  }

  resetReaction(rid) {
    this.assertPublishedStudyConstraints();
    this.invalidateSteadyCom();
    this.overrides.delete(rid);
    this.applyOverrides();
    this.solve();
  }

  resetAllReactions() {
    this.assertPublishedStudyConstraints();
    this.invalidateSteadyCom();
    this.overrides.clear();
    this.applyOverrides();
    this.solve();
  }

  /** Close a reaction (bounds 0, 0) and solve. */
  knockout(rid) {
    this.setReactionBounds(rid, 0, 0);
    this.solve();
  }

  /** Capture the current drawing before a multi-step edit such as dragging. */
  beginMapEdit(label = 'Edit map') {
    if (!this.map) return null;
    return { label, before: this.map.toJSON() };
  }

  /** Commit a previously captured edit if it actually changed the map. */
  commitMapEdit(transaction) {
    if (!transaction || !this.map) return false;
    const changed = this.mapHistory.record(
      transaction.before,
      this.map.toJSON(),
      transaction.label,
    );
    if (changed) this.emit('history');
    return changed;
  }

  get canUndoMap() { return this.mapHistory.canUndo; }
  get canRedoMap() { return this.mapHistory.canRedo; }

  // Saved / unsaved

  /** What was opened: switching it starts over with nothing to save. */
  workContextKey() {
    return JSON.stringify([
      this.scenario?.id ?? null,
      this.publishedCommunityId ?? null,
      this.isZunigaStudy ? this.zunigaScenarioId ?? null : null,
      this.species.map((sp) => [sp.id, sp.file]),
    ]);
  }

  /** What only the Scenario file holds and the user can change. */
  scenarioSignature() {
    return JSON.stringify({
      medium: this.medium,
      species: this.species.map((sp) => [sp.id, sp.enabled, sp.abundance]),
      overrides: [...this.overrides],
      objective: this.objective,
      productTarget: this.productTarget,
      parsimonious: this.parsimonious,
      pfbaBiomassPercent: this.pfbaBiomassPercent,
      loopless: this.loopless,
      fvaEnabled: this.fvaEnabled,
      steadyComConfig: this.steadyComConfig ?? null,
      layoutSettings: this.layoutSettings,
      reactionColorMode: this.view.reactionColorMode,
      lines: [this.view.widthByFlux, this.view.dashNoFlux, this.view.lineHops],
      ribbons: [this.view.pathwayRibbonMode, this.view.pathwayRibbonWidth, this.view.pathwayRibbonOpacity],
      omics: { settings: this.omicsSettings, dataset: objectRevision(this.omicsDataset) },
    });
  }

  /**
   * What Map JSON holds besides the drawing: pathways, flux colours and the
   * Zuñiga case. The Scenario file carries them too, so either export saves them.
   */
  sharedSignature() {
    return JSON.stringify({
      pathwayManager: this.pathwayManagerState(),
      fluxColorStops: this.view.fluxColorStops,
      study: this.zunigaState(),
    });
  }

  /**
   * An open has finished: what is on screen now is what was opened, so it is
   * all "saved". Every way of opening something calls this as its last step, so
   * defaults a loader sets late (the core-overview pathways, the colour mode of
   * a published case) are part of what was opened, not unsaved edits.
   */
  settleOpenedWork() {
    this.savedWork = null;
    this.trackWorkContext();
  }

  /**
   * The fallback for anything that changes the context without being one of
   * those opens: the starting point is taken when the store announces its
   * first change in the new context (see emit), not when the header is painted.
   */
  trackWorkContext() {
    const context = this.workContextKey();
    if (this.savedWork?.context === context) return;
    this.savedWork = {
      context,
      scenario: this.scenarioSignature(),
      shared: this.sharedSignature(),
    };
  }

  /**
   * Which parts of the work differ from what was last saved or opened.
   *
   * The map part follows the undo history, so undoing back to the saved map is
   * saved again, plus the pathways and colours the map file holds. The scenario
   * part compares the scenario settings with those of the last Scenario export
   * or import, or with what was opened. The model part asks whether a model in
   * use has edits not yet written out; that belongs to the model itself, so
   * opening another scenario that uses the same edited model does not hide it.
   * Nothing here changes any state.
   */
  unsavedWork() {
    const saved = this.savedWork;
    const known = saved?.context === this.workContextKey();
    return {
      map: Boolean(this.map) && (this.mapHistory.dirty || (known && this.sharedSignature() !== saved.shared)),
      scenario: known && this.scenarioSignature() !== saved.scenario,
      model: this.species.some((sp) => this.modelHasUnsavedEdits(this.models.get(sp.file))),
    };
  }

  /** The map on screen is now the one in a file: after Save map or opening one. */
  markMapSaved() {
    this.mapHistory.markSaved();
    this.trackWorkContext();
    this.savedWork.shared = this.sharedSignature();
  }

  /** The scenario settings, and what they share with the map file, are now in a file. */
  markScenarioSaved() {
    this.trackWorkContext();
    this.savedWork.scenario = this.scenarioSignature();
    this.savedWork.shared = this.sharedSignature();
  }

  /**
   * One model has been written out (the model editor exports the selected
   * species): its edits are saved, those of any other model are not.
   */
  markModelSaved(model = this.modelEditorModel()) {
    if (model) this.modelEditsSaved.set(model, this.modelEdits.get(model) ?? 0);
  }

  modelHasUnsavedEdits(model) {
    return Boolean(model) && (this.modelEdits.get(model) ?? 0) !== (this.modelEditsSaved.get(model) ?? 0);
  }

  undoMap() {
    if (!this.map) return null;
    const entry = this.mapHistory.undo(this.map.toJSON());
    if (!entry) return null;
    this.map = EditableMap.fromJSON(entry.doc);
    this.selectedReaction = null;
    this.selectedEntity = null;
    this.variability = null;
    this.emit('map');
    return entry;
  }

  redoMap() {
    if (!this.map) return null;
    const entry = this.mapHistory.redo(this.map.toJSON());
    if (!entry) return null;
    this.map = EditableMap.fromJSON(entry.doc);
    this.selectedReaction = null;
    this.selectedEntity = null;
    this.variability = null;
    this.emit('map');
    return entry;
  }

  /** How many objects the multi-selection holds. */
  get multiSelectionSize() {
    return selectionSize(this.multiSelection);
  }

  multiSelectionBucket(kind) {
    if (kind === 'reaction') return this.multiSelection.reactions;
    if (kind === 'text') return this.multiSelection.texts;
    return this.multiSelection.nodes;
  }

  isMultiSelected(kind, id) {
    return this.multiSelectionBucket(kind).has(id);
  }

  clearMultiSelection({ emit = true } = {}) {
    if (!this.multiSelectionSize) return false;
    this.multiSelection = emptySelection();
    if (emit) this.emit('multi-selection');
    return true;
  }

  /** Add an object to the multi-selection or take it out. `kind` is 'node', 'reaction' or 'text'. */
  toggleMultiSelection(kind, id) {
    if (!id || !['node', 'reaction', 'text'].includes(kind)) return this.multiSelection;
    const bucket = this.multiSelectionBucket(kind);
    if (bucket.has(id)) bucket.delete(id);
    else bucket.add(id);
    this.emit('multi-selection');
    return this.multiSelection;
  }

  /** Select what lies in a map rectangle (nodes, reactions, texts, whole compartments), adding to the selection unless `replace`. */
  selectInRect(rect, { replace = true } = {}) {
    if (!this.map) return this.multiSelection;
    const found = this.map.selectableInRect(rect, { includeSecondary: this.view.showCurrency !== false });
    if (replace) this.multiSelection = emptySelection();
    for (const id of found.nodes) this.multiSelection.nodes.add(id);
    for (const id of found.reactions) this.multiSelection.reactions.add(id);
    for (const id of found.texts ?? []) this.multiSelection.texts.add(id);
    this.multiSelection.compartments ??= new Set();
    for (const key of found.compartments ?? []) this.multiSelection.compartments.add(key);
    this.emit('multi-selection');
    return this.multiSelection;
  }

  // Part of a drag: the selection moves together and carries the compartments
  // in it (dragSelection), so an organelle can be moved with what is round it.
  moveMultiSelection(dx, dy) {
    return this.map?.dragSelection(this.multiSelection, dx, dy) ?? null;
  }

  moveCompartmentLabel(cellId, compartmentId, x, y) {
    this.assertMapOperation('moveCompartmentLabel');
    return this.map?.setCompartmentLabelPosition(cellId, compartmentId, x, y) ?? null;
  }

  /** The map-wide label source for 'metabolite' or 'reaction' labels ('base', 'id', 'name', ...). */
  mapLabelSource(kind) {
    return this.map?.labelSettings?.[kind] ?? (kind === 'metabolite' ? 'base' : 'id');
  }

  mapLabelObject(kind, id) {
    if (kind === 'metabolite') {
      return this.map?.nodes.get(id);
    }
    return this.map?.reactions.get(id);
  }

  mapLabelEntity(kind, object) {
    if (kind === 'metabolite') {
      return this.community?.merged.metabolites.get(object?.mergedId);
    }
    return this.community?.merged.reactions.get(object?.id);
  }

  /** The label a drawn node or reaction shows: by `source`, else its own source, else the map's. */
  mapLabelText(kind, id, source = null) {
    const object = this.mapLabelObject(kind, id);
    if (!object) {
      return '';
    }
    const entity = this.mapLabelEntity(kind, object);
    const selectedSource = source ?? object.labelSource ?? this.mapLabelSource(kind);
    return objectLabel(object, entity, kind, selectedSource);
  }

  /**
   * A metabolite or reaction of the model named as the map names it, drawn or
   * not: by the label source the map is set to (Map options ▸ Labels). A menu
   * that offers things to add reads the way the map does, so a yeast drawn
   * with names is not offered as `s_0034`.
   *
   * @returns {{label:string, id:string, name:string}} the label, the model's
   *   own id and its name, for menus that show one beside the other
   */
  entityLabel(kind, mergedId) {
    const metabolite = kind === 'metabolite';
    const entity = metabolite
      ? this.community?.merged.metabolites.get(mergedId)
      : this.community?.merged.reactions.get(mergedId);
    const owner = (metabolite ? this.community?.metaboliteOwner : this.community?.reactionOwner)?.get(mergedId);
    // The fields a drawn node or reaction carries, so both are labelled alike.
    const object = {
      id: mergedId,
      localId: owner?.originalId ?? entity?.id ?? mergedId,
      cellId: owner?.speciesId ?? null,
      name: entity?.name,
      ...(metabolite ? { base: localMetaboliteBase(this.community, mergedId, entity) } : {}),
    };
    const sources = objectLabelSources(object, entity, kind);
    return {
      label: objectLabel(object, entity, kind, this.mapLabelSource(kind)),
      id: sources.id,
      name: entity?.name ?? '',
    };
  }

  /** The label sources on offer, as {value, label}: for one drawn object, or for the whole map without `id`. */
  mapLabelOptions(kind, id = null) {
    const keys = new Set(kind === 'metabolite' ? ['base', 'id', 'name'] : ['id', 'name']);
    if (id) {
      const object = this.mapLabelObject(kind, id);
      if (object) {
        const entity = this.mapLabelEntity(kind, object);
        for (const key of Object.keys(objectLabelSources(object, entity, kind))) {
          keys.add(key);
        }
      }
    } else {
      const entities = kind === 'metabolite'
        ? this.community?.merged.metabolites
        : this.community?.merged.reactions;
      for (const entity of entities?.values() ?? []) {
        for (const key of Object.keys(entity.labelSources ?? {})) {
          keys.add(key);
        }
      }
    }
    const options = [];
    for (const key of keys) {
      options.push({ value: key, label: labelSourceName(key) });
    }
    return options;
  }

  /** Label one drawn object, or the whole map without `id`, by `source` ('inherit' clears an object's own). One undoable edit. */
  setMapLabelSource(kind, source, id = null) {
    this.assertMapOperation('setMapLabelSource');
    if (!this.map || !['metabolite', 'reaction'].includes(kind)) {
      return false;
    }
    const options = this.mapLabelOptions(kind, id);
    if (source !== 'inherit' && !options.some(option => option.value === source)) {
      return false;
    }
    const history = this.beginMapEdit('Change map labels');
    if (id) {
      const object = this.mapLabelObject(kind, id);
      if (!object) {
        return false;
      }
      if (source === 'inherit') {
        delete object.labelSource;
      } else {
        object.labelSource = source;
      }
    } else {
      if (source === 'inherit') {
        return false;
      }
      this.map.labelSettings ??= {};
      this.map.labelSettings[kind] = source;
    }
    const changed = this.commitMapEdit(history);
    this.emit('map-labels');
    return changed;
  }

  /** Every drawn node's or reaction's label, by id. */
  mapLabels(kind) {
    const objects = kind === 'metabolite' ? this.map?.nodes : this.map?.reactions;
    const labels = new Map();
    for (const id of objects?.keys() ?? []) {
      labels.set(id, this.mapLabelText(kind, id));
    }
    return labels;
  }


  /** Remove the current multi-selection from the drawing only, as one undoable edit. */
  removeMultiSelection() {
    this.assertMapOperation('removeMultiSelection');
    if (!this.map || !this.multiSelectionSize) return null;

    const before = this.map.toJSON();
    const reactionIds = [...this.multiSelection.reactions];
    const nodeIds = [...this.multiSelection.nodes];
    const textIds = [...this.multiSelection.texts];
    const removed = { reactions: [], nodes: [], texts: [], compartments: [] };

    // Whole compartments first, with what is drawn in them: what is left of
    // the selection after that is what lies outside them.
    for (const key of this.multiSelection.compartments ?? []) {
      const where = parseCompartmentKey(key);
      if (!where?.cellId) continue;
      const result = this.map.removeCompartment(where.cellId, where.compartmentId);
      if (!result) continue;
      removed.compartments.push(...result.compartmentIds);
      removed.reactions.push(...result.reactions);
      removed.texts.push(...result.texts);
    }
    // Reactions next: their reaction-local visual nodes may disappear with
    // them. Then remove only still-existing, explicitly selected node instances.
    for (const reactionId of reactionIds) {
      const result = this.map.removeReaction(reactionId);
      if (result) removed.reactions.push(reactionId);
    }
    for (const nodeId of nodeIds) {
      if (!this.map.nodes.has(nodeId) || !this.map.canRemoveNode(nodeId)) continue;
      if (this.map.removeNode(nodeId)) removed.nodes.push(nodeId);
    }
    // Annotations go with the rest, so deleting a pathway does not leave its
    // metabolite names behind, labelling reactions that are no longer there.
    for (const textId of textIds) {
      if (this.map.removeText(textId)) removed.texts.push(textId);
    }

    if (!removed.reactions.length && !removed.nodes.length && !removed.texts.length
      && !removed.compartments.length) return null;
    this.mapHistory.record(before, this.map.toJSON(), 'Remove selection');
    this.multiSelection = emptySelection();
    if (removed.compartments.length && this.selectedGeometry
      && !this.map.geometryTarget(this.selectedGeometry.cellId, this.selectedGeometry.compartmentId)) {
      this.selectedGeometry = null;
    }
    if (this.selectedReaction && removed.reactions.includes(this.selectedReaction)) {
      this.selectedReaction = null;
      this.variability = null;
    }
    if (this.selectedEntity) {
      const selectedRemoved = (this.selectedEntity.type === 'reaction'
        && removed.reactions.includes(this.selectedEntity.id))
        || (this.selectedEntity.type === 'metabolite'
          && !this.map.nodes.has(this.selectedEntity.id))
        || (this.selectedEntity.type === 'text'
          && removed.texts.includes(this.selectedEntity.id))
        || (this.selectedEntity.type === 'compartment' && !this.selectedGeometry)
        || (this.selectedEntity.type === 'cell'
          && !this.map.cells.some((cell) => cell.id === this.selectedEntity.id));
      if (selectedRemoved) this.selectedEntity = null;
    }
    this.emit('multi-selection');
    this.emit('map');
    return removed;
  }

  /** Whether the selection can be arranged: two objects, or one reaction (Escher-style line). */
  canArrangeSelection(action = 'align-vertical') {
    if (!this.map) return false;
    if (this.multiSelectionSize >= 2) return true;
    return (action === 'align-horizontal' || action === 'align-vertical')
      && this.map.selectionHasLineReactions(this.multiSelection);
  }

  /** Align or distribute the selection ('align-left', 'distribute-vertical', ...) as one undoable edit. */
  arrangeMultiSelection(action) {
    if (!this.canArrangeSelection(action)) return null;

    const labels = {
      'align-horizontal': 'Align horizontally',
      'align-vertical': 'Align vertically',
      'align-left': 'Align left',
      'align-right': 'Align right',
      'align-top': 'Align top',
      'align-bottom': 'Align bottom',
      'distribute-horizontal': 'Distribute horizontally',
      'distribute-vertical': 'Distribute vertically',
    };
    const label = labels[action];
    if (!label) return null;

    const before = this.map.toJSON();
    const mode = action.slice(action.indexOf('-') + 1);
    // With reactions selected, Horizontal and Vertical work as in Escher: each
    // reaction comes with its metabolites and they all end on one line.
    // Transporters stay on their membranes, so a selection with no other
    // reaction is aligned as plain objects.
    const asLine = (mode === 'horizontal' || mode === 'vertical') && action.startsWith('align-')
      && this.map.selectionHasLineReactions(this.multiSelection);
    const result = asLine
      ? this.map.alignSelectionInLine(this.multiSelection, mode)
      : action.startsWith('align-')
        ? this.map.alignSelection(this.multiSelection, mode)
        : this.map.distributeSelection(this.multiSelection, mode);
    if (result?.held?.length) {
      this.status = {
        ...this.status,
        state: 'warn',
        message: `${result.held.length} metabolite${result.held.length === 1 ? '' : 's'} could not reach the line: a metabolite stays inside its compartment.`,
      };
    }

    const changed = this.mapHistory.record(before, this.map.toJSON(), label);
    if (changed) this.emit('map');
    else if (result?.held?.length) this.emit('status');
    return { ...result, changed };
  }

  /**
   * Replace what is drawn, without touching the model or the solution.
   *
   * This is the seam a map editor plugs into: the community, the LP and the
   * current fluxes are all untouched, so swapping in a different subset of the
   * same reactions redraws the screen and changes no number.
   */
  setMap(map) {
    this.map = map;
    this.multiSelection = emptySelection();
    if (this.selectedReaction && !this.map?.hasReaction(this.selectedReaction)) {
      this.selectedReaction = null;
      this.selectedEntity = null;
      this.variability = null;
    }
    this.emit('map');
    return this.map;
  }

  /**
   * Add one reaction to the current EditableMap. Graphical only: no rebuild,
   * solve or relayout of existing geometry.
   */
  addMapReaction(reactionId, opts = {}) {
    this.assertMapOperation('addMapReaction');
    if (!this.map || !this.community) return null;
    const before = this.map.toJSON();
    const result = this.map.addReaction(this.community, reactionId, opts);
    if (!result) return null;
    this.mapHistory.record(before, this.map.toJSON(), 'Add reaction');
    this.emit('map');
    return result;
  }

  /** Remove one reaction from the drawing, never from the model or LP. */
  removeMapReaction(reactionId) {
    this.assertMapOperation('removeMapReaction');
    if (!this.map) return null;
    const before = this.map.toJSON();
    const result = this.map.removeReaction(reactionId);
    if (!result) return null;
    this.mapHistory.record(before, this.map.toJSON(), 'Remove reaction');
    if (this.selectedReaction === reactionId) {
      this.selectedReaction = null;
      this.selectedEntity = null;
      this.variability = null;
    }
    this.multiSelection.reactions.delete(reactionId);
    for (const id of result.nodes ?? []) this.multiSelection.nodes.delete(id);
    this.emit('map');
    return result;
  }

  /**
   * Delete a compartment from the drawing, with the compartments nested in it
   * and everything drawn in them (a cell's outermost one takes the cell). One
   * undoable step; the model is unchanged, and the compartment can be drawn
   * again from Add Compartment.
   */
  removeMapCompartment(cellId, compartmentId) {
    this.assertMapOperation('removeMapCompartment');
    if (!this.map) return null;
    const before = this.map.toJSON();
    const result = this.map.removeCompartment(cellId, compartmentId);
    if (!result) return null;
    this.mapHistory.record(before, this.map.toJSON(), result.cellRemoved ? 'Remove cell' : 'Remove compartment');

    this.selectedGeometry = null;
    if (['compartment', 'cell'].includes(this.selectedEntity?.type)
      || (this.selectedEntity?.type === 'reaction' && result.reactions.includes(this.selectedEntity.id))
      || (this.selectedEntity?.type === 'metabolite' && !this.map.nodes.has(this.selectedEntity.id))) {
      this.selectedEntity = null;
    }
    if (this.selectedReaction && !this.map.reactions.has(this.selectedReaction)) {
      this.selectedReaction = null;
      this.variability = null;
    }
    this.multiSelection = emptySelection();

    const what = [
      result.compartmentIds.length > 1 && `${result.compartmentIds.length - 1} nested compartment${result.compartmentIds.length > 2 ? 's' : ''}`,
      result.reactions.length && `${result.reactions.length} reaction${result.reactions.length === 1 ? '' : 's'}`,
      result.nodes.length && `${result.nodes.length} metabolite${result.nodes.length === 1 ? '' : 's'}`,
      result.texts?.length && `${result.texts.length} text${result.texts.length === 1 ? '' : 's'}`,
    ].filter(Boolean);
    this.status = {
      ...this.status,
      state: 'ok',
      message: `Removed ${result.cellRemoved ? 'the cell ' : ''}${result.label} from the map`
        + `${what.length ? `, with ${what.join(', ')}` : ''}. The model is unchanged; Ctrl+Z brings it back.`,
    };
    this.emit('map');
    return result;
  }

  /** Replace the drawing with a truly blank white canvas. */
  showEmptyMap() {
    if (!this.community) return null;
    this.mapMode = 'empty';
    const before = this.map?.toJSON() ?? null;
    const width = this.map?.width ?? 1200;
    const height = this.map?.height ?? 800;
    const next = EditableMap.blank({ width, height });
    if (before) this.mapHistory.record(before, next.toJSON(), 'Empty map');
    this.selectedReaction = null;
    this.selectedGeometry = null;
    this.selectedEntity = null;
    this.variability = null;
    return this.setMap(next);
  }

  /** Restore the complete automatic map for the current biological model. */
  showFullMap() {
    if (!this.community) return null;
    const before = this.map?.toJSON() ?? null;
    const next = EditableMap.fromCommunity(this.community, layoutEngineOptions(this.layoutSettings));
    if (before) this.mapHistory.record(before, next.toJSON(), 'Full map');
    this.mapMode = 'full';
    this.removeCoreOverviewPathways();
    this.selectedReaction = null;
    this.selectedEntity = null;
    this.variability = null;
    return this.setMap(next);
  }

  coreMetabolismOverviewSupport() {
    return coreOverviewSupport(this.community);
  }

  /** Replace the drawing with the central carbon metabolism overview, with its pathways as manual ones. Returns the build, or null when unsupported. */
  showCoreMetabolismOverview({ recordHistory = true } = {}) {
    if (!this.community) return null;
    const support = coreOverviewSupport(this.community);
    if (!support.supported) {
      this.status = {
        ...this.status,
        state: 'warn',
        message: 'This community does not contain enough annotated central-carbon reactions for the core metabolism overview.',
      };
      this.emit('status');
      return null;
    }

    const built = buildCoreMetabolismOverview(this.community, this.layoutSettings);
    if (!built?.map) return null;

    const before = this.map?.toJSON() ?? null;
    if (recordHistory && before) {
      this.mapHistory.record(before, built.map.toJSON(), 'Core metabolism overview');
    } else {
      this.mapHistory.clear();
    }

    this.map = built.map;
    this.mapMode = 'core-overview';
    this.installCoreOverviewPathways(built.pathwayGroups);
    // Keep the glycolysis view visually clean: no pathway ribbons, no
    // cross-feeding arcs and no helper/cofactor nodes at overview scale.
    this.view.pathwayRibbonMode = 'off';
    this.view.showCrossFeeding = false;
    this.view.showCurrency = true;
    this.selectedReaction = null;
    this.selectedGeometry = null;
    this.selectedEntity = null;
    this.multiSelection = emptySelection();
    this.variability = null;
    this.status = {
      ...this.status,
      state: 'ok',
      message: `Glycolysis overview: ${built.placed} reactions arranged vertically from G6P to pyruvate.`,
    };
    this.emit('map');
    return built;
  }

  /** The overview's pathway groups as manual pathways with a `core-` id, replacing earlier ones. */
  installCoreOverviewPathways(groups = []) {
    this.removeCoreOverviewPathways();
    const aggregated = new Map();
    for (const group of groups) {
      const current = aggregated.get(group.groupId) ?? {
        id: `core-${group.groupId}`,
        name: group.name,
        reactionIds: [],
      };
      current.reactionIds.push(...group.reactionIds);
      aggregated.set(group.groupId, current);
    }
    for (const group of aggregated.values()) {
      this.manualPathways.set(group.id, {
        id: group.id,
        name: group.name,
        color: categoryColor(group.name, this.view.dark),
        visible: true,
        reactionIds: new Set(group.reactionIds),
      });
    }
  }

  removeCoreOverviewPathways() {
    for (const id of [...this.manualPathways.keys()]) {
      if (id.startsWith('core-')) this.manualPathways.delete(id);
    }
  }

  /** A truly blank drawing surface for the active biological community. */
  emptyMap() {
    if (!this.community) return null;
    return EditableMap.blank({
      width: this.map?.width ?? 1200,
      height: this.map?.height ?? 800,
    });
  }

  /** Species/cells that are not yet drawn on the current map. */
  availableMapCells() {
    if (!this.map) return [];
    const drawn = new Set(this.map.cells.map((cell) => cell.id));
    const out = [];

    for (const sp of this.species.filter((item) => item.enabled !== false && !drawn.has(item.id))) {
      const model = this.models.get(sp.file);
      if (!model) continue;
      const { roots, byId } = model.compartmentTree();
      const outer = byId.get('e')?.parent === null
        ? byId.get('e')
        : roots[0];
      if (!outer) continue;

      out.push({
        speciesId: sp.id,
        speciesName: model.cell?.name ?? sp.id,
        organism: model.cell?.organism ?? '',
        outerCompartmentId: outer.id,
        outerCompartmentName: cellCompartmentName(outer.id, model.compartmentNames),
      });
    }

    return out.sort((a, b) => a.speciesName.localeCompare(b.speciesName));
  }

  setCellPlacementSpecies(speciesId) {
    const available = this.availableMapCells();
    const next = available.find((item) => item.speciesId === speciesId)?.speciesId
      ?? available[0]?.speciesId
      ?? null;
    this.cellPlacementSpeciesId = next;
    this.emit('cell-placement');
    return next;
  }

  /** Draw a species' cell (its outermost compartment) in `rect`, as one undoable edit, and select it. */
  addMapCell(speciesId, rect) {
    if (!this.map) return null;
    const available = this.availableMapCells();
    const spec = available.find((item) => item.speciesId === speciesId)
      ?? available.find((item) => item.speciesId === this.cellPlacementSpeciesId)
      ?? available[0];
    if (!spec) return null;

    const sp = this.species.find((item) => item.id === spec.speciesId);
    const model = this.models.get(sp?.file);
    if (!sp || !model) return null;

    const before = this.map.toJSON();
    const result = this.map.addCell({
      speciesId: sp.id,
      outerCompartmentId: spec.outerCompartmentId,
      outerLabel: spec.outerCompartmentName,
      cellName: model.cell?.name ?? sp.id,
      organism: model.cell?.organism ?? '',
      note: model.cell?.note ?? '',
      color: model.cell?.color ?? '#64748b',
      abundance: sp.abundance,
      rect,
    });
    if (!result) return null;

    this.mapHistory.record(before, this.map.toJSON(), 'Create cell');
    this.cellPlacementSpeciesId = this.availableMapCells()[0]?.speciesId ?? null;
    this.selectedReaction = null;
    this.selectedGeometry = {
      cellId: result.cell.id,
      compartmentId: result.cell.outerCompartment,
    };
    this.selectedEntity = { type: 'cell', id: result.cell.id };
    this.emit('map');
    return result;
  }

  selectCellGeometry(cellId) {
    const cell = this.map?.cells.find((item) => item.id === cellId);
    const outer = cell?.compartments.find((item) => item.id === cell.outerCompartment)
      ?? cell?.compartments.find((item) => item.parentId === null);
    if (!cell || !outer) return null;

    this.selectedReaction = null;
    this.selectedGeometry = { cellId: cell.id, compartmentId: outer.id };
    this.selectedEntity = { type: 'cell', id: cell.id };
    this.variability = null;
    this.emit('geometry-selection');
    return this.selectedGeometry;
  }

  /**
   * Compartments that may be added next without violating the source model's
   * containment tree. A child becomes available after its parent is drawn.
   */
  availableMapCompartments() {
    if (!this.map) return [];
    const out = [];
    for (const sp of this.species.filter((item) => item.enabled !== false)) {
      const model = this.models.get(sp.file);
      if (!model) continue;
      const cell = this.map.cells.find((c) => c.id === sp.id);
      if (!cell) continue;
      const drawn = new Set(cell.compartments?.map((c) => c.id) ?? []);
      const { byId } = model.compartmentTree();

      for (const node of byId.values()) {
        if (drawn.has(node.id)) continue;
        // Root/extracellular geometry is created by the Cell tool. Manual
        // compartment creation starts below that cell boundary.
        if (!node.parent) continue;
        if (!drawn.has(node.parent)) continue;
        out.push({
          speciesId: sp.id,
          speciesName: model.cell?.name ?? sp.id,
          compartmentId: node.id,
          compartmentName: cellCompartmentName(node.id, model.compartmentNames),
          parentId: node.parent ?? null,
          depth: node.depth,
        });
      }
    }
    return out.sort((a, b) =>
      a.speciesName.localeCompare(b.speciesName)
      || a.depth - b.depth
      || a.compartmentId.localeCompare(b.compartmentId)
    );
  }

  /**
   * Every compartment of every active species, drawn or not, and whether it
   * can be drawn now. `status` is 'ready' (its parent is on the map), 'waiting'
   * (its parent is not, yet), 'drawn', or 'cell' (it is a cell's outermost
   * compartment, which the Cell tool draws); `reason` says so in words.
   */
  mapCompartmentChoices() {
    if (!this.map) return [];
    const out = [];
    for (const sp of this.species.filter((item) => item.enabled !== false)) {
      const model = this.models.get(sp.file);
      if (!model) continue;
      const cell = this.map.cells.find((c) => c.id === sp.id);
      const drawn = new Set(cell?.compartments?.map((c) => c.id) ?? []);
      const { byId } = model.compartmentTree();
      const name = (id) => cellCompartmentName(id, model.compartmentNames);

      for (const node of byId.values()) {
        let status;
        let reason;
        if (drawn.has(node.id)) {
          status = 'drawn';
          reason = 'already on the map';
        } else if (!node.parent) {
          status = 'cell';
          reason = 'draw the cell with Add Cell';
        } else if (!cell) {
          status = 'waiting';
          reason = 'draw the cell first';
        } else if (drawn.has(node.parent)) {
          status = 'ready';
          reason = `goes inside ${name(node.parent)}`;
        } else {
          status = 'waiting';
          reason = `goes inside ${name(node.parent)}: draw that first`;
        }
        out.push({
          speciesId: sp.id,
          speciesName: model.cell?.name ?? sp.id,
          compartmentId: node.id,
          compartmentName: name(node.id),
          parentId: node.parent ?? null,
          parentName: node.parent ? name(node.parent) : null,
          depth: node.depth,
          status,
          reason,
        });
      }
    }
    return out.sort((a, b) =>
      a.speciesName.localeCompare(b.speciesName)
      || a.depth - b.depth
      || a.compartmentId.localeCompare(b.compartmentId)
    );
  }

  setCompartmentShapePreset(name) {
    const next = name === 'auto' || COMPARTMENT_SHAPES[name] ? name : 'auto';
    if (next === this.compartmentShapePreset) return;
    this.compartmentShapePreset = next;
    this.emit('compartment-placement');
  }

  setCompartmentPlacement(key) {
    const available = this.availableMapCompartments();
    const next = available.find((item) => `${item.speciesId}|${item.compartmentId}` === key)
      ?? available[0]
      ?? null;
    const nextKey = next ? `${next.speciesId}|${next.compartmentId}` : null;
    const changed = this.compartmentPlacementKey !== nextKey;
    this.compartmentPlacementKey = nextKey;
    if (changed) this.emit('compartment-placement');
    return next;
  }

  currentCompartmentPlacement() {
    const available = this.availableMapCompartments();
    const selected = available.find((item) =>
      `${item.speciesId}|${item.compartmentId}` === this.compartmentPlacementKey
    ) ?? available[0] ?? null;
    if (selected && !this.compartmentPlacementKey) {
      this.compartmentPlacementKey = `${selected.speciesId}|${selected.compartmentId}`;
    }
    return selected;
  }

  compartmentParentRect(speciesId, compartmentId) {
    const available = this.availableMapCompartments();
    const spec = available.find((item) =>
      item.speciesId === speciesId && item.compartmentId === compartmentId
    );
    if (!spec?.parentId) return null;
    const cell = this.map?.cells.find((item) => item.id === speciesId);
    const parent = cell?.compartments.find((item) => item.id === spec.parentId);
    return parent ? { ...parent.rect } : null;
  }

  /** Draw one compartment inside its drawn parent, with the current shape preset, as one undoable edit. */
  addMapCompartment(speciesId, compartmentId, rect = null) {
    if (!this.map) return null;
    const sp = this.species.find((item) => item.id === speciesId && item.enabled !== false);
    const model = this.models.get(sp?.file);
    if (!sp || !model) return null;

    const treeNode = model.compartmentTree().byId.get(compartmentId);
    const cell = this.map.cells.find((item) => item.id === speciesId);
    if (!treeNode || !cell || !treeNode.parent) return null;

    const before = this.map.toJSON();
    const result = this.map.addCompartment({
      speciesId,
      compartmentId,
      parentId: treeNode.parent ?? null,
      label: cellCompartmentName(compartmentId, model.compartmentNames),
      cellName: model.cell?.name ?? speciesId,
      organism: model.cell?.organism ?? '',
      note: model.cell?.note ?? '',
      color: model.cell?.color ?? '#64748b',
      abundance: sp.abundance,
      rect,
    });
    if (!result) return null;
    const shape = COMPARTMENT_SHAPES[this.compartmentShapePreset];
    if (shape) this.map.editCompartmentShape(speciesId, compartmentId, shape);

    this.mapHistory.record(before, this.map.toJSON(), 'Agregar compartimento');
    this.selectedReaction = null;
    this.selectedGeometry = { cellId: speciesId, compartmentId };
    this.selectedEntity = { type: 'compartment', id: compartmentId, cellId: speciesId };
    const next = this.availableMapCompartments()[0] ?? null;
    this.compartmentPlacementKey = next
      ? `${next.speciesId}|${next.compartmentId}`
      : null;
    this.emit('map');
    return result;
  }

  /** What setMapCanvasSize would settle on, without changing anything. */
  previewCanvasSize(width, height) {
    if (!this.map) return null;
    const min = this.map.minimumCanvasSize();
    return {
      width: Math.max(min.width, Number(width) || this.map.canvas.width),
      height: Math.max(min.height, Number(height) || this.map.canvas.height),
    };
  }

  setMapCanvasSize(width, height, move) {
    if (!this.map) return null;
    return this.map.setCanvasSize(width, height, move);
  }

  /** Apply a layout preset by laying the map out again, or rebuilding the overview when that is what is drawn. */
  setLayoutPreset(name) {
    this.layoutSettings = layoutPreset(name);
    if (this.mapMode === 'core-overview') {
      return this.showCoreMetabolismOverview({ recordHistory: true });
    }
    return this.relayoutMap(`Layout preset: ${this.layoutSettings.preset}`);
  }

  /** Change some automatic layout settings and lay the map out again. */
  setLayoutSettings(patch = {}) {
    this.layoutSettings = normalizeLayoutSettings({
      ...this.layoutSettings,
      ...patch,
      preset: patch.preset ?? this.layoutSettings.preset,
    });
    if (this.mapMode === 'core-overview') {
      return this.showCoreMetabolismOverview({ recordHistory: true });
    }
    return this.relayoutMap('Adjust automatic layout');
  }

  /**
   * Why the automatic layout must not be re-run over the current drawing, or
   * null when it may.
   *
   * A layout preset means "lay the map out again". For a model too large to be
   * drawn automatically the map on screen was built by hand, reaction by
   * reaction. Laying it out again first places every reaction of the model,
   * which takes minutes with the page frozen, and then throws the hand-placed
   * positions away. Nobody pressing "Compact" in the toolbar is asking for that.
   */
  relayoutBlocked() {
    if (!this.community || !this.map) return null;
    const drawn = this.map.reactions.size;
    if (!drawn || drawn >= this.community.merged.reactions.size) return null;
    return this.tooLargeToAutoDraw()
      ? 'Layout presets re-run the automatic layout, which would replace this hand-built drawing and take minutes on a model this size. The drawing was kept.'
      : null;
  }

  /** A species too large for the automatic layout to place in reasonable time. */
  tooLargeToAutoDraw() {
    return Boolean(this.community?.activeSpecies.some((sp) =>
      sp.model.reactions.size > AUTO_DRAW_REACTION_LIMIT
      || sp.model.metabolites.size > AUTO_DRAW_METABOLITE_LIMIT));
  }

  /**
   * What to ask before drawing the full model, or null when it is cheap.
   *
   * Drawing everything is a legitimate request, so it is not refused like a
   * layout preset; but on a genome-scale model it freezes the page for minutes
   * and replaces the drawing, and the user should hear that before, not after.
   */
  fullMapWarning() {
    if (!this.community || !this.tooLargeToAutoDraw()) return null;
    const total = this.community.merged.reactions.size;
    const drawn = this.map?.reactions.size ?? 0;
    return `Drawing all ${total} reactions runs the automatic layout on a genome-scale model; `
      + 'the page may stop responding for several minutes.'
      + (drawn ? ` It replaces the current drawing of ${drawn} reactions (Undo restores it).` : '')
      + '\n\nDraw the full model?';
  }

  /**
   * Re-run only the graphical automatic layout. The model and current solution
   * are untouched. If the map is a reaction subset, keep that same subset.
   */
  relayoutMap(label = 'Relayout map') {
    if (!this.community || !this.map) return null;
    const drawnReactionIds = this.map.reactionIds();
    const wasBlank = this.map.cells.length === 0 && drawnReactionIds.length === 0;

    if (wasBlank) {
      this.emit('layout-settings');
      return this.map;
    }

    const blocked = this.relayoutBlocked();
    if (blocked) {
      this.status = { ...this.status, state: 'ok', message: blocked };
      this.emit('layout-settings');
      return this.map;
    }
    const before = this.map.toJSON();

    let next;
    if (!drawnReactionIds.length) {
      // Only the shell is drawn, so only the shell is laid out. Going through
      // the full layout and throwing every reaction away again would give the
      // same cells and compartments, after freezing the page for as long as a
      // genome-scale model takes to place.
      next = EditableMap.emptyFromCommunity(this.community, layoutEngineOptions(this.layoutSettings));
    } else {
      next = EditableMap.fromCommunity(
        this.community,
        layoutEngineOptions(this.layoutSettings),
      );
      if (drawnReactionIds.length < next.reactions.size) {
        const keep = drawnReactionIds.filter((id) => next.reactions.has(id));
        next = next.subsetByReactions(keep);
      }
    }

    this.mapHistory.record(before, next.toJSON(), label);
    this.map = next;
    this.selectedReaction = null;
    this.selectedGeometry = null;
    this.selectedEntity = null;
    this.multiSelection = emptySelection();
    this.variability = null;
    this.emit('layout-settings');
    return this.map;
  }

  /**
   * Move a metabolite node by hand.
   *
   * Purely graphical: the geometry of one node and the segments attached to it
   * change, and nothing else does. It deliberately neither solves nor rebuilds
   * nor relayouts, and it does not emit. Dragging repaints at its own pace, and
   * a store event here would drag the whole sidebar through every pointermove.
   *
   * @returns {object|null} the node that moved, or null if it is not movable
   */
  moveNode(nodeId, x, y) {
    return this.map ? this.map.moveNode(nodeId, x, y) : null;
  }

  /**
   * Complete an explicit drag-and-merge gesture. This intentionally does not
   * record history by itself: the surrounding node-drag transaction records
   * the move and merge as one undoable edit.
   */
  mergeOverlappingMetabolite(nodeId) {
    const result = this.map?.mergeOverlappingMetabolite(nodeId) ?? null;
    if (!result) return null;
    if (this.selectedEntity?.type === 'metabolite' && this.selectedEntity.id === nodeId) {
      this.selectedEntity = { ...this.selectedEntity, id: result.target.id };
    }
    return result;
  }

  /** Is this node one the user is allowed to move? */
  canMoveNode(nodeId) {
    return Boolean(this.map?.canMoveNode(nodeId));
  }

  canMoveReaction(reactionId) {
    return Boolean(this.map?.canMoveReaction(reactionId));
  }

  moveReaction(reactionId, x, y) {
    return this.map?.moveReaction(reactionId, x, y) ?? null;
  }

  moveBranchControl(reactionId, nodeId, handle, x, y) {
    return this.map?.moveBranchControl(reactionId, nodeId, handle, x, y) ?? null;
  }

  // Arranging one reaction

  canRotateReaction(reactionId) {
    return Boolean(this.map?.canRotateReaction(reactionId));
  }

  /**
   * Turn a reaction's backbone while it is being dragged round. Like moveNode,
   * this neither records history nor emits: the gesture that calls it does both
   * once, when it ends.
   */
  setReactionAngle(reactionId, angle) {
    return this.map?.setReactionAngle(reactionId, angle) ?? null;
  }

  /** One undoable change to how a single reaction is drawn. */
  editMapReaction(label, edit) {
    if (!this.map) return null;
    const before = this.map.toJSON();
    const result = edit(this.map);
    if (!result) return null;
    this.mapHistory.record(before, this.map.toJSON(), label);
    this.emit('map');
    return result;
  }

  /** Turn a reaction's backbone by a fixed step, as one undoable edit. */
  rotateMapReaction(reactionId, angle) {
    this.assertMapOperation('rotateMapReaction');
    return this.editMapReaction('Rotate reaction', (map) => map.rotateReaction(reactionId, angle));
  }

  /**
   * Keep a reaction straight between two of its metabolites, or release it.
   * Drawing only: the stoichiometry and the fluxes are untouched.
   */
  setMapReactionLinear(reactionId, linear) {
    this.assertMapOperation('setMapReactionLinear');
    const ends = linear ? this.selectedLineEnds(reactionId) : {};
    return this.editMapReaction(
      linear ? 'Make reaction straight' : 'Release straight reaction',
      (map) => map.setReactionLinear(reactionId, linear, ends),
    );
  }

  /**
   * The metabolites of this reaction that are selected on the map, as the two
   * its line should run through: one substrate, one product.
   *
   * Shift-click two metabolites of a reaction and the next Linear or compass
   * press uses those two. A side with none selected, or with more than one, is
   * left for the reaction to decide.
   */
  selectedLineEnds(reactionId) {
    const picked = this.multiSelection?.nodes;
    if (!this.map || !picked?.size) return {};
    const sides = this.map.reactionSides(reactionId);
    const one = (entries) => {
      const chosen = entries.filter((entry) => picked.has(entry.nodeId));
      return chosen.length === 1 ? chosen[0].nodeId : undefined;
    };
    const ends = {};
    const from = one(sides.substrates);
    const to = one(sides.products);
    if (from) ends.from = from;
    if (to) ends.to = to;
    return ends;
  }

  /** Choose one of the two metabolites a reaction's line runs through. */
  setMapReactionLineEnd(reactionId, nodeId) {
    this.assertMapOperation('setMapReactionLineEnd');
    return this.editMapReaction('Choose line metabolite', (map) =>
      map.setReactionLineEnd(reactionId, nodeId));
  }

  /**
   * Lay a reaction out along a compass direction, substrate to product, with
   * its two line metabolites on either side of the marker.
   */
  alignMapReaction(reactionId, direction) {
    this.assertMapOperation('alignMapReaction');
    const ends = this.selectedLineEnds(reactionId);
    return this.editMapReaction(`Align reaction ${direction}`, (map) =>
      map.alignReaction(reactionId, direction, ends));
  }

  /**
   * Escher's Align vertical / Align horizontal for one reaction: lay it and its
   * two line metabolites on a vertical or horizontal line through its marker,
   * keeping the way it runs now (a product below its substrate stays below).
   */
  alignMapReactionAxis(reactionId, axis) {
    if (!this.map || (axis !== 'vertical' && axis !== 'horizontal')) return null;
    const ends = { ...this.map.reactionLineEnds(reactionId), ...this.selectedLineEnds(reactionId) };
    const from = this.map.nodes.get(ends.from);
    const to = this.map.nodes.get(ends.to);
    if (!from || !to) return null;
    const direction = axis === 'vertical'
      ? (to.y >= from.y ? 'S' : 'N')
      : (to.x >= from.x ? 'E' : 'W');
    return this.alignMapReaction(reactionId, direction);
  }

  /** Put a reaction's cofactors and curves back in their base shape. */
  tidyMapReaction(reactionId) {
    this.assertMapOperation('tidyMapReaction');
    return this.editMapReaction('Tidy reaction', (map) => map.tidyReaction(reactionId));
  }

  /** Exchange the places of two metabolites on one side of a reaction. */
  swapMapReactionMetabolites(reactionId, nodeIdA, nodeIdB) {
    this.assertMapOperation('swapMapReactionMetabolites');
    return this.editMapReaction('Swap metabolites', (map) =>
      map.swapReactionMetabolites(reactionId, nodeIdA, nodeIdB));
  }

  /** Exchange the substrate side and the product side of a reaction's drawing. */
  flipMapReaction(reactionId) {
    this.assertMapOperation('flipMapReaction');
    return this.editMapReaction('Swap substrates and products', (map) => map.flipReaction(reactionId));
  }

  /** Mirror a reaction's drawing in its own backbone. */
  mirrorMapReaction(reactionId) {
    this.assertMapOperation('mirrorMapReaction');
    return this.editMapReaction('Mirror reaction', (map) => map.mirrorReaction(reactionId));
  }

  /** New text is set in `style`, by default the last size and font chosen. */
  addMapText(text, x, y, style = this.view.textStyle) {
    if (!this.map) return null;
    const before = this.map.toJSON();
    const annotation = this.map.addText(text, x, y, { fontSize: style?.fontSize, fontFamily: style?.fontFamily });
    if (!annotation) return null;
    this.mapHistory.record(before, this.map.toJSON(), 'Add text');
    this.emit('map');
    return annotation;
  }

  /** Change a text's words and, when given, its size and font: one undo step. */
  updateMapText(id, text, style = null) {
    if (!this.map) return null;
    const before = this.map.toJSON();
    const annotation = this.map.updateText(id, text);
    if (!annotation) return null;
    if (style) this.map.styleText(id, style);
    this.mapHistory.record(before, this.map.toJSON(), 'Edit text');
    this.emit('map');
    return annotation;
  }

  /**
   * Set a text's size and/or font. The choice also becomes the style of the
   * next new text, as in any editor.
   */
  styleMapText(id, style = {}) {
    if (!this.map) return null;
    const before = this.map.toJSON();
    const annotation = this.map.styleText(id, style);
    if (!annotation) return null;
    this.setTextStyle({ fontSize: annotation.fontSize, fontFamily: annotation.fontFamily });
    this.mapHistory.record(before, this.map.toJSON(), 'Text style');
    this.emit('map');
    return annotation;
  }

  /** The size and font new texts are set in. */
  setTextStyle({ fontSize, fontFamily } = {}) {
    const current = this.view.textStyle ?? { fontSize: TEXT_SIZE.default, fontFamily: DEFAULT_TEXT_FONT };
    this.view.textStyle = {
      fontSize: fontSize === undefined || fontSize === null || fontSize === '' ? current.fontSize : clampTextSize(fontSize),
      fontFamily: fontFamily === undefined || fontFamily === null ? current.fontFamily : normalizeTextFont(fontFamily),
    };
    return this.view.textStyle;
  }

  moveMapText(id, x, y) {
    return this.map?.moveText(id, x, y) ?? null;
  }

  removeMapText(id) {
    if (!this.map) return null;
    const before = this.map.toJSON();
    const annotation = this.map.removeText(id);
    if (!annotation) return null;
    this.mapHistory.record(before, this.map.toJSON(), 'Eliminar texto');
    if (this.selectedEntity?.type === 'text' && this.selectedEntity.id === id) this.selectedEntity = null;
    this.emit('map');
    return annotation;
  }

  /** Draw another instance of a metabolite node, as one undoable edit. */
  duplicateMapMetabolite(nodeId, opts = {}) {
    this.assertMapOperation('duplicateMapMetabolite');
    if (!this.map) return null;
    const before = this.map.toJSON();
    const node = this.map.duplicateMetaboliteNode(nodeId, opts);
    if (!node) return null;
    this.mapHistory.record(before, this.map.toJSON(), 'Duplicate metabolite');
    this.emit('map');
    return node;
  }

  metaboliteCopies(nodeId) {
    const node = this.map?.nodes.get(nodeId);
    if (!node?.mergedId) return [];
    return this.map.nodesForMetabolite(node.mergedId);
  }

  /**
   * Turn a metabolite's label while it is dragged. No history and no emit: the
   * drag gesture owns the repaint and its single history transaction.
   */
  setMetaboliteLabelAngle(nodeId, angle) {
    this.assertMapOperation('setMetaboliteLabelAngle');
    return this.map?.setMetaboliteLabelAngle(nodeId, angle) ?? null;
  }

  /** Mark a metabolite node as primary or secondary (cofactor-like), as one undoable edit. */
  setMapNodePrimary(nodeId, primary, opts = {}) {
    if (!this.map) return null;
    const before = this.map.toJSON();
    const node = this.map.setNodePrimary(nodeId, primary, opts);
    if (!node) return null;
    this.mapHistory.record(
      before,
      this.map.toJSON(),
      node.primary ? 'Mark metabolite as primary' : 'Mark metabolite as secondary',
    );
    this.emit('map');
    return node;
  }

  /**
   * Colour a cell or one of its compartments on the map: map-only, undoable and
   * saved with the map. `colors` holds `fill` and/or `stroke`; null restores the
   * theme colour.
   */
  setMapGeometryColors(cellId, compartmentId, colors) {
    if (!this.map) return null;
    const before = this.map.toJSON();
    const target = compartmentId
      ? this.map.setCompartmentColors(cellId, compartmentId, colors)
      : this.map.setCellColors(cellId, colors);
    if (!target) return null;
    this.mapHistory.record(before, this.map.toJSON(), compartmentId ? 'Compartment colours' : 'Cell colours');
    this.emit('map');
    return target;
  }

  setMapNodeRadius(nodeId, radius) {
    if (!this.map) return null;
    const before = this.map.toJSON();
    const node = this.map.setNodeRadius(nodeId, radius);
    if (!node) return null;
    this.mapHistory.record(before, this.map.toJSON(), 'Resize metabolite');
    this.emit('map');
    return node;
  }

  setMapNodeHidden(nodeId, hidden = true) {
    if (!this.map) return null;
    const before = this.map.toJSON();
    const node = this.map.setNodeHidden(nodeId, hidden);
    if (!node) return null;
    this.mapHistory.record(before, this.map.toJSON(), hidden ? 'Hide metabolite' : 'Show metabolite');
    this.emit('map');
    return node;
  }

  /** The colours of every metabolite circle without its own: `{ fill, stroke }`, null for the theme's. */
  setMapNodeStyle(colors) {
    if (!this.map) return null;
    const before = this.map.toJSON();
    const style = this.map.setNodeStyle(colors);
    if (!this.mapHistory.record(before, this.map.toJSON(), 'Metabolite colours')) return style;
    this.emit('map');
    return style;
  }

  /** Colour some metabolite circles (a selection, or one): `{ fill, stroke }`, null for the map's. */
  setMapNodeColors(nodeIds, colors) {
    if (!this.map) return [];
    const before = this.map.toJSON();
    const changed = this.map.setNodeColors([...nodeIds], colors);
    if (!changed.length || !this.mapHistory.record(before, this.map.toJSON(), 'Metabolite colour')) return changed;
    this.emit('map');
    return changed;
  }

  /** The colour some reactions are drawn in when lines are not coloured by flux; null for their pathway's. */
  setMapReactionColor(reactionIds, color) {
    if (!this.map) return [];
    const before = this.map.toJSON();
    const changed = this.map.setReactionColor([...reactionIds], color);
    if (!changed.length || !this.mapHistory.record(before, this.map.toJSON(), 'Reaction colour')) return changed;
    this.emit('map');
    return changed;
  }

  get hiddenMetaboliteCount() {
    return this.map?.hiddenNodeCount?.() ?? 0;
  }

  showAllHiddenMetabolites() {
    if (!this.map) return [];
    const before = this.map.toJSON();
    const changed = this.map.showAllHiddenNodes();
    if (!changed.length) return changed;
    this.mapHistory.record(before, this.map.toJSON(), 'Show hidden metabolites');
    this.emit('map');
    return changed;
  }

  /**
   * Search the complete biological reaction set, including reactions not drawn.
   * Results are ranked by ID first (the model's, or the label the map shows),
   * then name and subsystem.
   */
  searchReactions(query, { limit = 30 } = {}) {
    if (!this.community) return [];
    const q = normalizeSearch(query);
    if (!q) return [];

    const results = [];
    for (const [reactionId, reaction] of this.community.merged.reactions) {
      const owner = this.community.reactionOwner?.get(reactionId) ?? {};
      const localId = owner.originalId ?? reaction.id ?? reactionId;
      const name = reaction.name ?? '';
      const subsystem = reaction.subsystem ?? '';
      const speciesId = owner.speciesId ?? '';
      const { label } = this.entityLabel('reaction', reactionId);

      const fields = {
        fullId: normalizeSearch(reactionId),
        localId: normalizeSearch(localId),
        label: normalizeSearch(label),
        name: normalizeSearch(name),
        subsystem: normalizeSearch(subsystem),
      };

      let score = Infinity;
      if (fields.localId === q || fields.fullId === q || fields.label === q) score = 0;
      else if (fields.localId.startsWith(q) || fields.label.startsWith(q)) score = 1;
      else if (fields.fullId.startsWith(q)) score = 2;
      else if (fields.name.startsWith(q)) score = 3;
      else if (fields.localId.includes(q) || fields.label.includes(q)) score = 4;
      else if (fields.fullId.includes(q)) score = 5;
      else if (fields.name.includes(q)) score = 6;
      else if (fields.subsystem.includes(q)) score = 7;
      if (!Number.isFinite(score)) continue;

      results.push({
        reactionId,
        localId,
        label,
        // Beside the label, whichever of the name and the model id it is not.
        detail: label === name ? (localId === label ? '' : localId) : name,
        name,
        subsystem,
        speciesId,
        drawn: Boolean(this.map?.hasReaction(reactionId)),
        score,
      });
    }

    results.sort((a, b) =>
      a.score - b.score
      || String(a.label).localeCompare(String(b.label))
      || String(a.speciesId).localeCompare(String(b.speciesId))
    );
    return results.slice(0, Math.max(1, Number(limit) || 30));
  }

  /** Reactions in the biological model that touch a drawn metabolite but are not drawn yet. */
  reactionsAvailableFromMetabolite(nodeId) {
    const node = this.map?.nodes.get(nodeId);
    if (!node?.mergedId || !this.community) return [];
    const out = [];
    for (const [reactionId, reaction] of this.community.merged.reactions) {
      if (this.map.hasReaction(reactionId)) continue;
      if (Object.prototype.hasOwnProperty.call(reaction.metabolites, node.mergedId)) out.push(reactionId);
    }
    return out;
  }

  /** The model's metabolites in one compartment of a species, labelled as the map labels them, with how often each is drawn. */
  metabolitesAvailableInCompartment(cellId, compartmentId) {
    if (!this.community || !this.map) return [];
    const out=[];
    for (const [id,metabolite] of this.community.merged.metabolites) {
      const owner=this.community.metaboliteOwner?.get(id);
      if(owner?.speciesId!==cellId || owner?.compartment!==compartmentId) continue;
      const named=this.entityLabel('metabolite',id);
      // Beside the map's label, whichever of the name and the model id it is not.
      const detail=named.label===named.name?named.id:named.name;
      out.push({id,label:named.label,name:detail,modelId:named.id,count:this.map.nodesForMetabolite(id).length});
    }
    return out.sort((a,b)=>a.label.localeCompare(b.label));
  }
  /** Draw a new instance of a merged metabolite (at `opts.x`, `opts.y`), as one undoable edit. */
  addMapMetabolite(mergedId, opts={}) {
    this.assertMapOperation('addMapMetabolite');
    if(!this.map||!this.community)return null;
    const before=this.map.toJSON(),node=this.map.addMetabolite(this.community,mergedId,opts);
    if(!node)return null;
    this.mapHistory.record(before,this.map.toJSON(),'Add metabolite');this.emit('map');return node;
  }

  /** Internal reactions available in one visible cellular compartment. */
  reactionsAvailableInCompartment(cellId, compartmentId) {
    if (!this.community || !this.map) return [];
    return reactionsInCompartment(this.community, cellId, compartmentId)
      .filter((reactionId) => !this.map.hasReaction(reactionId));
  }

  canRemoveMapNode(nodeId) {
    return Boolean(this.map?.canRemoveNode(nodeId));
  }

  /** Remove one metabolite node from the drawing, never from the model, as one undoable edit. */
  removeMapNode(nodeId) {
    this.assertMapOperation('removeMapNode');
    if (!this.map) return null;
    const before = this.map.toJSON();
    const node = this.map.removeNode(nodeId);
    if (!node) return null;
    this.mapHistory.record(before, this.map.toJSON(), 'Eliminar metabolito');
    if (this.selectedEntity?.type === 'metabolite' && this.selectedEntity.id === nodeId) this.selectedEntity = null;
    this.emit('map');
    return node;
  }

  /**
   * Select a compartment's geometry, which is what puts its five handles on
   * screen.
   *
   * It does not depend on the geometry-editing view: Select edits geometry too
   * (moving a compartment, resizing it, dragging the shared medium), so the
   * only question is whether the thing being selected exists.
   */
  selectGeometry(cellId, compartmentId) {
    if (!this.map?.geometryTarget(cellId, compartmentId)) {
      this.selectedGeometry = null;
      this.selectedEntity = null;
      this.emit('geometry-selection');
      return null;
    }
    this.selectedGeometry = { cellId, compartmentId };
    this.selectedReaction = null;
    this.selectedEntity = { type: 'compartment', id: compartmentId, cellId };
    this.variability = null;
    this.emit('geometry-selection');
    return this.selectedGeometry;
  }

  clearGeometrySelection() {
    if (!this.selectedGeometry) return;
    this.selectedGeometry = null;
    if (this.selectedEntity?.type === 'compartment') this.selectedEntity = null;
    this.emit('geometry-selection');
  }

  editGeometryRect(cellId, compartmentId, rect) {
    return this.map?.editGeometryRect(cellId, compartmentId, rect) ?? null;
  }

  /** Reshape while a shape handle is dragged; the drag records the history. */
  editCompartmentShape(cellId, compartmentId, shape) {
    return this.map?.editCompartmentShape(cellId, compartmentId, shape) ?? null;
  }

  /** A compartment's shape in one undoable step, as the inspector's presets set it. */
  setCompartmentShape(cellId, compartmentId, shape) {
    if (!this.map) return null;
    const before = this.map.toJSON();
    const result = this.map.editCompartmentShape(cellId, compartmentId, shape);
    if (!result) return null;
    this.mapHistory.record(before, this.map.toJSON(), 'Compartment shape');
    this.emit('map');
    return result;
  }

  /** Select a reaction by merged id (null clears), with its FVA range when FVA is on. */
  select(rid) {
    this.selectedReaction = rid;
    this.selectedEntity = rid ? { type: 'reaction', id: rid } : null;
    this.variability = this.fvaEnabled && rid
      ? this.fvaRanges.get(rid) ?? null
      : null;
    this.emit('selection');
  }

  // Omics overlay

  /** Read a gene or protein omics CSV for the overlay. Two or more conditions start in compare mode. */
  importOmicsCSV(text, { filename = 'omics.csv', dataType = 'gene' } = {}) {
    const dataset = parseOmicsCSV(text, { source: filename, dataType });
    this.omicsDataset = dataset;
    this.omicsSettings = {
      ...this.omicsSettings,
      enabled: true,
      dataType,
      mode: dataset.conditions.length >= 2 ? 'compare' : 'single',
      conditionA: dataset.conditions[0] ?? null,
      conditionB: dataset.conditions[1] ?? dataset.conditions[0] ?? null,
    };
    this.emit('omics');
    return dataset;
  }

  clearOmics({ emit = true } = {}) {
    this.omicsDataset = null;
    this.omicsSettings = {
      enabled: false,
      dataType: this.omicsSettings?.dataType ?? 'gene',
      mode: 'single',
      conditionA: null,
      conditionB: null,
      metric: 'log2fc',
      pseudocount: 1,
    };
    if (emit) this.emit('omics');
  }

  setOmicsSettings(patch = {}) {
    const next = { ...this.omicsSettings, ...patch };
    if (!['gene', 'protein'].includes(next.dataType)) next.dataType = 'gene';
    if (!['single', 'compare'].includes(next.mode)) next.mode = 'single';
    if (!['log2fc', 'difference'].includes(next.metric)) next.metric = 'log2fc';
    next.pseudocount = Math.max(0, Number(next.pseudocount) || 0);
    const conditions = this.omicsDataset?.conditions ?? [];
    if (!conditions.includes(next.conditionA)) next.conditionA = conditions[0] ?? null;
    if (!conditions.includes(next.conditionB)) next.conditionB = conditions[1] ?? conditions[0] ?? null;
    next.enabled = Boolean(next.enabled && this.omicsDataset);
    this.omicsSettings = next;
    this.emit('omics');
    return next;
  }

  omicsState() {
    return {
      dataset: serializeOmicsDataset(this.omicsDataset),
      settings: { ...this.omicsSettings },
    };
  }

  restoreOmicsState(raw = null) {
    this.omicsDataset = deserializeOmicsDataset(raw?.dataset);
    const conditions = this.omicsDataset?.conditions ?? [];
    this.omicsSettings = {
      enabled: Boolean(raw?.settings?.enabled && this.omicsDataset),
      dataType: ['gene', 'protein'].includes(raw?.settings?.dataType) ? raw.settings.dataType : 'gene',
      mode: ['single', 'compare'].includes(raw?.settings?.mode) ? raw.settings.mode : 'single',
      conditionA: conditions.includes(raw?.settings?.conditionA) ? raw.settings.conditionA : (conditions[0] ?? null),
      conditionB: conditions.includes(raw?.settings?.conditionB) ? raw.settings.conditionB : (conditions[1] ?? conditions[0] ?? null),
      metric: ['log2fc', 'difference'].includes(raw?.settings?.metric) ? raw.settings.metric : 'log2fc',
      pseudocount: Math.max(0, Number(raw?.settings?.pseudocount) || 0),
    };
  }

  /** Reaction colours and values for the omics overlay: condition A, or A against B, with how many drawn reactions match. */
  omicsOverlay() {
    const dataset = this.omicsDataset;
    const settings = this.omicsSettings;
    const fallbackColor = this.view.dark ? '#475569' : '#cbd5e1';
    const totalDrawn = this.map?.reactionIds?.().length ?? 0;
    const empty = {
      enabled: false, colors: new Map(), values: new Map(),
      scoresA: new Map(), scoresB: new Map(), matchedReactions: 0,
      totalDrawn, scale: null, conditionA: settings.conditionA,
      conditionB: settings.conditionB, mode: settings.mode,
      metric: settings.metric, fallbackColor,
    };
    if (!dataset || !settings.enabled || !settings.conditionA || !this.community) return empty;

    const scoresA = reactionOmicsScores(
      this.community, this.species, this.models, dataset, settings.conditionA,
    );
    let scoresB = new Map();
    let values = scoresA;
    let scale;

    if (settings.mode === 'compare' && settings.conditionB) {
      scoresB = reactionOmicsScores(
        this.community, this.species, this.models, dataset, settings.conditionB,
      );
      values = compareReactionScores(scoresA, scoresB, {
        metric: settings.metric,
        pseudocount: settings.pseudocount,
      });
      scale = divergingOmicsColors(values, { dark: this.view.dark });
    } else {
      scale = sequentialOmicsColors(values, { dark: this.view.dark });
    }

    const drawn = new Set(this.map?.reactionIds?.() ?? []);
    const matchedReactions = [...values.keys()].filter((rid) => drawn.has(rid)).length;
    return {
      enabled: true, colors: scale.colors, values, scoresA, scoresB,
      matchedReactions, totalDrawn: drawn.size, scale,
      conditionA: settings.conditionA, conditionB: settings.conditionB,
      mode: settings.mode, metric: settings.metric, fallbackColor,
    };
  }

  // Pathway manager

  pathwayManagerState() {
    return {
      manual: [...this.manualPathways.values()].map((pathway) => ({
        id: pathway.id,
        name: pathway.name,
        color: pathway.color,
        visible: pathway.visible !== false,
        reactionIds: [...pathway.reactionIds],
      })),
      settings: [...this.pathwaySettings].map(([name, value]) => ({ name, ...value })),
    };
  }

  /** Restore manual pathways and model pathway settings from a saved state, dropping reactions the model does not have. */
  restorePathwayManagerState(state = null) {
    this.manualPathways = new Map();
    this.pathwaySettings = new Map();
    if (!state || typeof state !== 'object') return;

    for (const raw of state.manual ?? []) {
      const id = String(raw?.id ?? '').trim();
      const name = String(raw?.name ?? '').trim();
      if (!id || !name) continue;
      const reactionIds = new Set(
        (raw.reactionIds ?? []).filter((rid) => this.community?.merged?.reactions?.has(rid)),
      );
      if (!reactionIds.size) continue;
      this.manualPathways.set(id, {
        id,
        name,
        color: normalizeHexColor(raw.color) ?? categoryColor(name, this.view.dark),
        visible: raw.visible !== false,
        reactionIds,
      });
    }

    for (const raw of state.settings ?? []) {
      const name = String(raw?.name ?? '').trim();
      if (!name) continue;
      this.pathwaySettings.set(name, {
        visible: raw.visible !== false,
        ...(normalizeHexColor(raw.color) ? { color: normalizeHexColor(raw.color) } : {}),
      });
    }
  }

  /** A manual pathway of `reactionIds`, by default the selected reactions. Returns the new pathway. */
  createManualPathway({ name, color, reactionIds = null } = {}) {
    const label = String(name ?? '').trim();
    if (!label) throw new Error('Enter a pathway name.');

    const selected = reactionIds == null
      ? [...this.multiSelection.reactions]
      : [...reactionIds];
    const valid = [...new Set(selected)]
      .filter((rid) => this.community?.merged?.reactions?.has(rid));
    if (!valid.length) throw new Error('Select at least one reaction.');

    // A manual annotation is exclusive: assigning a reaction to a new manual
    // pathway removes it from the previous manual pathway, while the original
    // model pathway metadata remains untouched underneath.
    for (const [id, pathway] of this.manualPathways) {
      for (const rid of valid) pathway.reactionIds.delete(rid);
      if (!pathway.reactionIds.size) this.manualPathways.delete(id);
    }

    const id = uniquePathwayId(label, this.manualPathways);
    const pathway = {
      id,
      name: label,
      color: normalizeHexColor(color) ?? categoryColor(label, this.view.dark),
      visible: true,
      reactionIds: new Set(valid),
    };
    this.manualPathways.set(id, pathway);
    this.view.pathwayRibbonMode = 'pathway';
    this.view.reactionColorMode = 'flux';
    this.emit('pathways');
    return pathway;
  }

  updateManualPathway(id, patch = {}) {
    const pathway = this.manualPathways.get(id);
    if (!pathway) return null;
    const name = patch.name === undefined ? pathway.name : String(patch.name).trim();
    if (!name) throw new Error('The pathway name cannot be empty.');
    pathway.name = name;
    if (patch.color !== undefined) pathway.color = normalizeHexColor(patch.color) ?? pathway.color;
    if (patch.visible !== undefined) pathway.visible = Boolean(patch.visible);
    this.emit('pathways');
    return pathway;
  }

  deleteManualPathway(id) {
    if (!this.manualPathways.delete(id)) return false;
    this.emit('pathways');
    return true;
  }

  /** The drawn reactions of a pathway key, `manual:<id>` or `model:<name>`. */
  pathwayReactionIds(key) {
    const raw = String(key ?? '');
    if (raw.startsWith('manual:')) {
      const pathway = this.manualPathways.get(raw.slice(7));
      return pathway ? [...pathway.reactionIds].filter((rid) => this.map?.hasReaction(rid)) : [];
    }
    if (raw.startsWith('model:')) {
      const name = raw.slice(6);
      const base = buildReactionColoring(this.community?.merged, 'pathway', { dark: this.view.dark });
      return [...base.groups]
        .filter(([rid, group]) => group === name && this.map?.hasReaction(rid))
        .map(([rid]) => rid);
    }
    return [];
  }

  selectPathway(key) {
    const reactions = this.pathwayReactionIds(key);
    this.multiSelection = emptySelection();
    for (const rid of reactions) this.multiSelection.reactions.add(rid);
    this.selectedReaction = null;
    this.variability = null;
    this.view.multiSelect = true;
    this.emit('multi-selection');
    return reactions;
  }

  setPathwayVisible(key, visible) {
    const raw = String(key ?? '');
    if (raw.startsWith('manual:')) {
      return this.updateManualPathway(raw.slice(7), { visible });
    }
    if (raw.startsWith('model:')) {
      const name = raw.slice(6);
      const current = this.pathwaySettings.get(name) ?? {};
      this.pathwaySettings.set(name, { ...current, visible: Boolean(visible) });
      this.emit('pathways');
      return this.pathwaySettings.get(name);
    }
    return null;
  }

  setPathwayColor(key, color) {
    const next = normalizeHexColor(color);
    if (!next) return null;
    const raw = String(key ?? '');
    if (raw.startsWith('manual:')) {
      return this.updateManualPathway(raw.slice(7), { color: next });
    }
    if (raw.startsWith('model:')) {
      const name = raw.slice(6);
      const current = this.pathwaySettings.get(name) ?? {};
      this.pathwaySettings.set(name, { ...current, color: next });
      this.emit('pathways');
      return this.pathwaySettings.get(name);
    }
    return null;
  }

  /** The pathways listed in the pathway manager, manual first, each with its drawn reactions. */
  pathwayManagerEntries() {
    if (!this.community?.merged || !this.map) return [];
    const drawn = new Set(this.map.reactionIds());
    const base = buildReactionColoring(
      this.community.merged,
      'pathway',
      { dark: this.view.dark, reactionIds: drawn },
    );

    const entries = [];
    for (const item of base.legend) {
      if (/^Sin pathway$/i.test(item.label)) continue;
      const reactionIds = [...base.groups]
        .filter(([rid, group]) => drawn.has(rid) && group === item.label)
        .map(([rid]) => rid);
      if (!reactionIds.length) continue;
      const settings = this.pathwaySettings.get(item.label) ?? {};
      entries.push({
        key: `model:${item.label}`,
        id: item.label,
        name: item.label,
        source: 'model',
        color: settings.color ?? item.color,
        visible: settings.visible !== false,
        reactionIds,
        count: reactionIds.length,
      });
    }

    for (const pathway of this.manualPathways.values()) {
      const reactionIds = [...pathway.reactionIds].filter((rid) => drawn.has(rid));
      entries.push({
        key: `manual:${pathway.id}`,
        id: pathway.id,
        name: pathway.name,
        source: 'manual',
        color: pathway.color,
        visible: pathway.visible !== false,
        reactionIds,
        count: reactionIds.length,
      });
    }

    return entries.sort((a, b) => {
      if (a.source !== b.source) return a.source === 'manual' ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
  }

  /** Pathway colours with the manager's colours, hidden pathways and manual pathways applied: { mode, colors, groups, legend }. */
  managedPathwayColoring({ drawnOnly = false } = {}) {
    const reactionIds = drawnOnly ? (this.map?.reactionIds?.() ?? []) : null;
    const base = buildReactionColoring(
      this.community?.merged,
      'pathway',
      { dark: this.view.dark, reactionIds },
    );
    const colors = new Map(base.colors);
    const groups = new Map(base.groups);

    for (const [rid, group] of [...groups]) {
      const settings = this.pathwaySettings.get(group);
      if (settings?.visible === false) {
        groups.delete(rid);
        colors.delete(rid);
      } else if (settings?.color) {
        colors.set(rid, settings.color);
      }
    }

    for (const pathway of this.manualPathways.values()) {
      for (const rid of pathway.reactionIds) {
        if (!this.community?.merged?.reactions?.has(rid)) continue;
        if (pathway.visible === false) {
          groups.delete(rid);
          colors.delete(rid);
        } else {
          groups.set(rid, pathway.name);
          colors.set(rid, pathway.color);
        }
      }
    }

    const legend = legendFromManagedGroups(groups, colors, reactionIds);
    return { mode: 'pathway', colors, groups, legend };
  }

  /** How reaction lines are coloured when not by flux, by the view's colour mode. */
  reactionColoring({ drawnOnly = false } = {}) {
    if (this.view.reactionColorMode === 'pathway') {
      return this.managedPathwayColoring({ drawnOnly });
    }
    const reactionIds = drawnOnly ? (this.map?.reactionIds?.() ?? []) : null;
    return buildReactionColoring(
      this.community?.merged,
      this.view.reactionColorMode,
      { dark: this.view.dark, reactionIds },
    );
  }

  /** The colouring of the pathway ribbons behind reactions, by the view's ribbon mode. */
  pathwayRibbonColoring({ drawnOnly = false } = {}) {
    const mode = normalizePathwayRibbonMode(this.view.pathwayRibbonMode);
    if (mode === 'off') return { mode, colors: new Map(), groups: new Map(), legend: [] };
    if (mode === 'pathway') return this.managedPathwayColoring({ drawnOnly });
    const reactionIds = drawnOnly ? (this.map?.reactionIds?.() ?? []) : null;
    return buildReactionColoring(
      this.community?.merged,
      mode,
      { dark: this.view.dark, reactionIds },
    );
  }

  /** Select any map object for the inspector: a reaction, metabolite, text, cell or compartment. */
  selectEntity(type, id, details = {}) {
    if (!type || !id) {
      this.select(null);
      return null;
    }
    this.selectedReaction = type === 'reaction' ? id : null;
    this.selectedGeometry = null;
    this.variability = this.fvaEnabled && type === 'reaction'
      ? this.fvaRanges.get(id) ?? null
      : null;
    this.selectedEntity = { type, id, ...details };
    this.emit('selection');
    return this.selectedEntity;
  }

  /** Refresh presentation only; this never changes the model or solves an LP. */
  refreshFluxScale() {
    const fluxes = this.displayFluxes().values();
    const reference = this.zunigaState()?.fluxReference;
    const differenceStops = this.isZunigaStudy && this.zunigaComparison === 'difference'
      ? [{ value: -reference, color: '#2166ac' }, { value: 0, color: '#bdbdbd' }, { value: reference, color: '#b2182b' }] : null;
    this.scale = new FluxScale(fluxes, {
      dark: this.view.dark,
      colorStops: differenceStops ?? this.view.fluxColorStops,
    });
    if (this.isZunigaStudy) this.scale.reference = reference;
    return this.scale;
  }

  setFluxColorStops(stops) {
    this.setView({ fluxColorStops: stops });
    return this.view.fluxColorStops;
  }

  /** Change display settings, normalising the values that need it, and emit 'view'. */
  setView(patch) {
    if ('scrollInput' in patch) {
      patch = { ...patch, scrollInput: ['auto', 'mouse', 'touchpad'].includes(patch.scrollInput) ? patch.scrollInput : 'auto' };
    }
    if ('fluxColorStops' in patch) {
      patch = { ...patch, fluxColorStops: normalizeFluxColorStops(patch.fluxColorStops) };
    }
    if ('reactionColorMode' in patch) {
      patch = { ...patch, reactionColorMode: normalizeReactionColorMode(patch.reactionColorMode) };
    }
    if ('pathwayRibbonMode' in patch) {
      patch = { ...patch, pathwayRibbonMode: normalizePathwayRibbonMode(patch.pathwayRibbonMode) };
    }
    for (const key of ['widthByFlux', 'dashNoFlux', 'lineHops']) {
      if (key in patch) patch = { ...patch, [key]: Boolean(patch[key]) };
    }
    if ('pathwayRibbonWidth' in patch) {
      patch = { ...patch, pathwayRibbonWidth: clampNumber(patch.pathwayRibbonWidth, 20, 120, 56) };
    }
    if ('pathwayRibbonOpacity' in patch) {
      patch = { ...patch, pathwayRibbonOpacity: clampNumber(patch.pathwayRibbonOpacity, 0.04, 0.45, 0.18) };
    }
    Object.assign(this.view, patch);
    if ('geometryEdit' in patch && !patch.geometryEdit) this.selectedGeometry = null;
    if ('geometryEdit' in patch && patch.geometryEdit) {
      this.selectedReaction = null;
      this.selectedEntity = null;
      this.variability = null;
    }
    if ('dark' in patch || 'fluxColorStops' in patch) {
      this.refreshFluxScale();
    }
    this.emit('view');
  }

  // Map I/O

  /**
   * What exported files are named after. A single-species model goes by its
   * own id (`yeastGEM_v9.1.1`), which is what the person loaded; a community by
   * its scenario or published-community id (`consorcio-anaerobio`); several
   * uploaded models by their ids together.
   */
  exportBaseName() {
    const modelId = (sp) => this.models.get(sp.file)?.id || sp.id;
    if (this.species.length === 1) return modelId(this.species[0]);
    const scenarioId = String(this.scenario?.id ?? '').replace(/^published:/, '');
    if (scenarioId && scenarioId !== 'uploaded-models') return scenarioId;
    if (this.species.length) {
      const ids = this.species.slice(0, 3).map(modelId);
      return ids.join('+') + (this.species.length > 3 ? `+${this.species.length - 3}` : '');
    }
    return this.community?.name || 'coxeter';
  }

  /** `<exportBaseName>_<suffix>.<extension>`, e.g. `yeastGEM_v9.1.1_map.json`. */
  fileNameFor(suffix, extension) {
    return exportFileName(this.exportBaseName(), suffix, extension);
  }

  /**
   * Hover text for an exported SVG: what a reaction or a metabolite is, named
   * as the map names it, with its model id, and what the solution puts
   * through a reaction.
   *
   * @returns {{reaction:(id:string)=>string, node:(id:string)=>string}}
   */
  exportTooltips() {
    const solved = this.solution?.status === 'optimal';
    const fluxes = this.displayFluxes?.() ?? this.solution?.fluxes ?? new Map();
    const named = (kind, id) => {
      const { label, id: modelId, name } = this.entityLabel(kind, id);
      const lines = [label];
      if (name && name !== label) lines.push(name);
      if (modelId && modelId !== label) lines.push(`ID: ${modelId}`);
      return lines;
    };
    return {
      reaction: (rid) => {
        const lines = named('reaction', rid);
        const reaction = this.community?.merged.reactions.get(rid);
        if (reaction?.subsystem) lines.push(`Subsystem: ${reaction.subsystem}`);
        const bounds = this.currentReactionBounds(rid);
        // Bounds as written in the model: 1000, not 1.00e+3.
        const bound = (v) => (Number.isFinite(v) ? String(Number(v.toPrecision(6))) : String(v));
        if (bounds) lines.push(`Bounds: [${bound(bounds.lower)}, ${bound(bounds.upper)}]`);
        lines.push(solved ? `Flux: ${formatFlux(fluxes.get(rid) ?? 0, 4)}` : 'Flux: not solved');
        return lines.join('\n');
      },
      node: (nodeId) => {
        const node = this.map?.nodes.get(nodeId);
        if (!node?.mergedId) return node?.base ?? nodeId;
        const lines = named('metabolite', node.mergedId);
        const metabolite = this.community?.merged.metabolites.get(node.mergedId);
        if (metabolite?.formula) lines.push(`Formula: ${metabolite.formula}`);
        if (node.compartment) lines.push(`Compartment: ${cellCompartmentName(node.compartment, this.models.get(this.species.find((sp) => sp.id === node.cellId)?.file)?.compartmentNames ?? {})}`);
        return lines.join('\n');
      },
    };
  }

  /**
   * What an exported SVG says about itself in its <metadata>: the program that
   * drew it, the model and analysis behind it, and how the map was drawn.
   */
  exportMetadata() {
    return {
      generator: 'Coxeter Multicell Flux Visualizer',
      software: softwareIdentity(),
      exportedAt: new Date().toISOString(),
      scenario: { id: this.scenario?.id ?? null, name: this.scenario?.name ?? null },
      species: this.species.filter((sp) => sp.enabled !== false).map((sp) => ({
        id: sp.id,
        model: this.models.get(sp.file)?.id ?? sp.file ?? null,
        abundance: sp.abundance ?? null,
      })),
      analysis: {
        method: this.lastRun?.label ?? (this.loopless ? 'Loopless FBA' : this.parsimonious ? 'pFBA' : 'FBA'),
        status: this.solution?.status ?? null,
        objective: this.objective,
        objectiveValue: Number.isFinite(this.solution?.objectiveValue) ? this.solution.objectiveValue : null,
      },
      map: {
        reactions: this.map?.reactions.size ?? 0,
        metabolites: this.map?.nodes.size ?? 0,
        labels: { metabolite: this.mapLabelSource('metabolite'), reaction: this.mapLabelSource('reaction') },
        lineColor: this.view.reactionColorMode,
        lineWidthByFlux: Boolean(this.view.widthByFlux),
        dashedWithoutFlux: this.view.dashNoFlux !== false,
        lineHops: this.view.lineHops !== false,
      },
      note: 'Exported map. Hover a reaction or metabolite for its identifiers and flux.',
    };
  }

  /** The Map JSON document: the drawing with its context, pathways, flux colours and Zuñiga state. */
  mapDocument() {
    if (!this.map) throw new Error('There is no active map to save.');
    return createMapDocument(this.map, {
      scenario: this.scenario?.id ?? null,
      species: this.species,
      pathwayManager: this.pathwayManagerState(),
      fluxColorStops: this.view.fluxColorStops,
      study: this.zunigaState(),
    });
  }

  mapJSON() {
    return JSON.stringify(this.mapDocument(), null, 2);
  }

  loadMapJSON(text) {
    this.mapLoadNote = '';
    return this.applyMapDocument(parseMapDocument(text));
  }

  /**
   * Open a saved map, switching to the context it was drawn over if need be.
   *
   * A map only means something over the model it was drawn on, so the file
   * records which scenario or published community that was. Someone who saves a
   * map, closes the page and opens the file again lands on the default scenario,
   * where the drawing does not belong, and refusing it there would turn the
   * most ordinary thing anyone does with a saved map into a dead end. Try the
   * active context first, so nothing moves when the file already fits, and
   * only then go to the one the file names.
   *
   * A map whose model is not here at all (one drawn over an imported model,
   * opened after the page was reloaded) is kept rather than refused, and
   * drawn as soon as that model is loaded (see drawWaitingMap). Resolves to
   * the map, or null when it is waiting.
   */
  async loadMapDocument(text, { name = null } = {}) {
    const parsed = parseMapDocument(text);
    this.mapLoadNote = '';
    // The map opened now replaces one that was still waiting for its model.
    this.waitingMap = null;
    if (parsed.study?.id === ZUNIGA_ID && !this.isZunigaStudy) await this.useZunigaStudy({ scenarioId: parsed.study.scenarioId });
    try {
      return await this.openMapDocument(parsed);
    } catch (error) {
      // A broken file, or the Zuñiga case with its own context, is not waiting
      // for anything.
      if (parsed.study || !hasMapStructure(parsed.map)) throw error;
      this.waitingMap = { parsed, name, needs: mapModelLabel(parsed), reason: error.message };
      this.addLog('warn', `Map ${name ?? ''} kept until its model is loaded: ${error.message}`.replace(/\s+/g, ' '));
      this.emit('waiting-map');
      return null;
    }
  }

  /**
   * Draw the map that was opened before its model was loaded, now that a
   * model has been. Called after each context is opened; only an import of
   * models, or the very scenario the map names, counts as loading its model.
   * A teaching scenario that happens to share reaction ids does not.
   *
   * @returns {boolean} whether the waiting map was drawn
   */
  drawWaitingMap() {
    const waiting = this.waitingMap;
    if (!waiting || !this.map || !this.community) return false;
    const here = this.scenario?.id ?? null;
    const imported = Boolean(this.scenario?.uploaded) || here === 'reproducibility-bundle';
    if (!imported && here !== (waiting.parsed.metadata?.scenario ?? null)) return false;

    // An import has just said what it loaded; say what was drawn after it.
    const opened = imported && this.status?.message ? `${this.status.message} ` : '';
    const file = waiting.name ? `“${waiting.name}”` : 'the map';
    this.mapLoadNote = '';
    try {
      try {
        this.applyMapDocument(waiting.parsed);
      } catch (error) {
        if (!this.adoptMapSpecies(waiting.parsed)) throw error;
      }
    } catch (error) {
      this.status = { ...this.status, state: 'warn', message: `${opened}${file} is still waiting: it does not fit this model (${error.message})` };
      this.emit('status');
      return false;
    }
    this.waitingMap = null;
    this.status = {
      ...this.status,
      state: this.mapLoadNote ? 'warn' : this.status?.state,
      message: `${opened}Drew ${file}, which was waiting for this model.${this.mapLoadNote}`,
    };
    this.addLog('ok', `Drew the waiting map ${waiting.name ?? ''} over ${here}`.replace(/\s+/g, ' '));
    this.emit('status');
    return true;
  }

  /** Forget a map that is waiting for its model. */
  discardWaitingMap() {
    if (!this.waitingMap) return;
    this.waitingMap = null;
    this.emit('waiting-map');
  }

  /** The body of loadMapDocument: fit the map to the active context or the one it names. */
  async openMapDocument(parsed) {
    try {
      return this.applyMapDocument(parsed);
    } catch (error) {
      const wanted = parsed.metadata?.scenario ?? null;
      if (wanted && wanted !== (this.scenario?.id ?? null)) {
        await this.restoreMapContext(wanted);
        return this.applyMapDocument(parsed);
      }
      // The right scenario is already open and the map still does not fit it:
      // the one thing left that is not a real mismatch is the same model under
      // another id.
      const adopted = this.adoptMapSpecies(parsed);
      if (adopted) return adopted;
      throw error;
    }
  }

  /**
   * Open a map that was drawn over the same model under another id.
   *
   * An uploaded model's species id is the model id its file declares, and that
   * changes from one export of a model to the next (`iCme_stage15` one day,
   * `sbml_model` the next). A map saved in between names every reaction by the
   * old one, so on its own it looks like "a different set of species". With
   * one species in the file and one in the session there is no doubt which is
   * meant, so the map is read against the active one. If its reactions are not
   * there the load fails, and says which.
   *
   * @returns {EditableMap|null} the map, or null when this is not that case
   */
  adoptMapSpecies(parsed) {
    const saved = mapSpeciesIds(parsed);
    const active = (this.species ?? []).filter((sp) => sp.enabled !== false);
    if (saved.length !== 1 || active.length !== 1 || saved[0] === active[0].id) return null;

    const renamed = renameMapSpecies(parsed, saved[0], active[0].id, {
      model: active[0].file ?? active[0].model ?? null,
    });
    // The cell is drawn with the name of the model it belongs to now.
    const cellName = this.models.get(active[0].file)?.cell?.name;
    if (cellName) for (const cell of renamed.map.cells ?? []) cell.name = cellName;

    let map;
    try {
      map = this.applyMapDocument(renamed);
    } catch (error) {
      throw new Error(`${error.message} (The map was saved for "${saved[0]}" and was read against "${active[0].id}", the model now open.)`);
    }
    this.mapLoadNote = ` It was saved for "${saved[0]}" and has been read against "${active[0].id}".${this.mapLoadNote ?? ''}`;
    return map;
  }

  /** Switch to the scenario or published community a saved map names. */
  async restoreMapContext(wanted) {
    const published = /^published:(.+)$/.exec(String(wanted ?? ''));
    if (published) {
      const id = published[1];
      const community = this.publishedCommunities?.find((c) => c.id === id);
      if (!community) {
        throw new Error(`This map belongs to the published community "${id}", which is not available here.`);
      }
      await this.usePublishedCommunity(id);
      if (!this.map) {
        throw new Error(`${community.name} bundles no models in this build, so there is nothing for the map to sit on.`);
      }
      return this.scenario;
    }

    const scenario = this.scenarios?.find((s) => s.id === wanted);
    if (!scenario) {
      throw new Error(`This map belongs to the scenario "${wanted}", which is not loaded.`);
    }
    await this.useScenario(wanted);
    return this.scenario;
  }

  /** Check a parsed map against the active model, fit it to the model and make it the drawing, as one undoable edit. */
  applyMapDocument(parsed) {
    const fluxColorStops = normalizeFluxColorStops(parsed.fluxColorStops ?? null);
    if (!this.community || !this.map) throw new Error('There is no active model to receive the map.');

    if (parsed.study?.id === ZUNIGA_ID) {
      if (!this.isZunigaStudy) throw new Error('Open the Zuñiga case before importing this map, or use the File menu to restore its context.');
      this.validateZunigaState(parsed.study);
    }

    validateMapContext(parsed, { species: this.species });
    const mapCommunity = this.isZunigaStudy ? createZunigaScenario(this.zunigaStudy, 'mutant-coculture').community : this.community;
    validateMapDocument(parsed.map, mapCommunity, this.species);
    if (parsed.study?.id === ZUNIGA_ID) {
      this.useZunigaScenario(parsed.study.scenarioId, { emit: false });
      this.zunigaComparison = parsed.study.comparison ?? 'absolute';
    }

    const before = this.map.toJSON();
    const next = EditableMap.fromJSON(parsed.map);
    // A map from an earlier export of the model, or from an earlier build of
    // this program, is brought up to date rather than refused: see
    // reconcile-map.js. A map that needs nothing comes through untouched.
    if (!this.isZunigaStudy) {
      // What a switched-off species has drawn is kept, not taken as lost.
      const keepSpecies = new Set(this.species.filter((sp) => !sp.enabled).map((sp) => sp.id));
      const contents = pruneMapToModel(next, this.community, { keepSpecies });
      const shell = reconcileCompartments(next, this.community);
      this.mapLoadNote = `${this.mapLoadNote ?? ''}${describeReconciliation(contents, shell)}`;
    }
    this.mapHistory.record(before, next.toJSON(), 'Load map JSON');
    this.map = next;
    this.view.fluxColorStops = fluxColorStops;
    this.refreshFluxScale();
    this.restorePathwayManagerState(parsed.pathwayManager);
    // What was just opened is what is in the file, pathways and colours
    // included. Undo still goes back to the previous drawing, which then
    // counts as unsaved.
    this.markMapSaved();
    this.selectedReaction = null;
    this.selectedEntity = null;
    this.variability = null;
    this.emit('map');
    return this.map;
  }

  // Exports

  /** The solution as CSV, one row per reaction with its flux and bounds. Only the header when there is no optimal solution. */
  fluxCSV() {
    const rows = ['reaction,species,local_id,name,subsystem,flux,lower_bound,upper_bound'];
    if (!this.solution || this.solution.status !== 'optimal') return rows.join('\n');
    for (const [rid, v] of this.solution.fluxes) {
      const owner = this.community.reactionOwner.get(rid) || {};
      const rxn = this.community.merged.reactions.get(rid);
      const fixed = this.solution.steadyCom;
      const rawId = owner.originalId ?? rid;
      const specific = fixed?.spec?.reactions?.get(rawId) ?? fixed?.spec?.members?.find((m) => m.id === owner.speciesId)?.model?.reactions.get(rawId);
      const b = specific ? { lower: specific.lowerBound, upper: specific.upperBound } : this.problem.getBounds(rid) || { lower: '', upper: '' };
      rows.push([
        csv(rid), csv(owner.speciesId ?? ''), csv(owner.originalId ?? ''),
        csv(rxn?.name ?? ''), csv(rxn?.subsystem ?? ''),
        v.toFixed(6), b.lower, b.upper,
      ].join(','));
    }
    return rows.join('\n');
  }

  /** The Scenario file: species, medium, overrides, analysis settings, display settings, pathways and omics. */
  stateJSON() {
    return JSON.stringify({
      scenario: this.scenario?.id ?? null,
      study: this.zunigaState(),
      steadyComConfig: this.steadyComConfig,
      steadyCom: this.solution?.steadyCom ? { mu: this.solution.steadyCom.mu } : null,
      objective: this.objective,
      productTarget: this.productTarget,
      parsimonious: this.parsimonious,
      pfbaBiomassPercent: this.pfbaBiomassPercent,
      loopless: this.loopless,
      fvaEnabled: this.fvaEnabled,
      layoutSettings: { ...this.layoutSettings },
      reactionColorMode: this.view.reactionColorMode,
      widthByFlux: this.view.widthByFlux,
      dashNoFlux: this.view.dashNoFlux,
      lineHops: this.view.lineHops,
      fluxColorStops: this.view.fluxColorStops,
      pathwayRibbonMode: this.view.pathwayRibbonMode,
      pathwayRibbonWidth: this.view.pathwayRibbonWidth,
      pathwayRibbonOpacity: this.view.pathwayRibbonOpacity,
      pathwayManager: this.pathwayManagerState(),
      omics: this.omicsState(),
      medium: this.medium,
      species: this.species,
      overrides: [...this.overrides].map(([id, b]) => ({ id, ...b })),
    }, null, 2);
  }

  /** Open a Scenario file: restore its context and settings, rebuild and solve again. */
  async loadState(text) {
    const doc = JSON.parse(text);
    const fluxColorStops = normalizeFluxColorStops(doc.fluxColorStops ?? null);
    if (doc.study?.id === ZUNIGA_ID || doc.scenario === `published:${ZUNIGA_ID}`) {
      if (!this.isZunigaStudy) await this.useZunigaStudy({ scenarioId: doc.study?.scenarioId ?? 'mutant-coculture' });
      if (doc.study) this.validateZunigaState(doc.study);
      this.useZunigaScenario(doc.study?.scenarioId ?? 'mutant-coculture', { emit: false });
      this.view.fluxColorStops = fluxColorStops;
      this.layoutSettings = normalizeLayoutSettings(doc.layoutSettings ?? this.layoutSettings);
      this.view.reactionColorMode = normalizeReactionColorMode(doc.reactionColorMode ?? 'flux');
      this.setZunigaComparison(doc.study?.comparison ?? 'absolute');
      this.restorePathwayManagerState(doc.pathwayManager);
      this.markScenarioSaved();
      this.emit('solution');
      return this;
    }
    if (doc.steadyCom && (!Number.isFinite(doc.steadyCom.mu) || doc.steadyCom.mu < 0)) throw new Error('The saved SteadyCom growth rate is invalid.');
    const fiber = doc.steadyComConfig?.fiber ?? null;
    if (fiber && (!Number.isFinite(fiber.bacteroides) || fiber.bacteroides < 1 || fiber.bacteroides > 30 || fiber.faecalibacterium !== 5)) {
      throw new Error('The saved SteadyCom fiber condition is invalid.');
    }
    this.invalidateSteadyCom();
    if (String(doc.scenario ?? '').startsWith('published:')) await this.usePublishedCommunity(doc.scenario.slice('published:'.length));
    this.steadyComConfig = { fiber, parsimonious: Boolean(doc.steadyComConfig?.parsimonious) };
    this.scenario = this.scenarios.find((s) => s.id === doc.scenario) ?? this.scenario;
    this.objective = doc.objective ?? OBJECTIVES.EQUAL_GROWTH;
    this.productTarget = doc.productTarget ?? null;
    this.parsimonious = Boolean(doc.parsimonious ?? doc.pfba ?? false);
    this.pfbaBiomassPercent = normalizePfbaPercent(
      doc.pfbaBiomassPercent
      ?? (doc.pfbaFraction !== undefined ? Number(doc.pfbaFraction) * 100 : 100)
    );
    this.loopless = Boolean(doc.loopless ?? doc.looplessFba ?? false);
    if (this.loopless) this.parsimonious = false;
    this.fvaEnabled = this.loopless
      ? false
      : Boolean(doc.fvaEnabled ?? doc.fva ?? false);
    this.fvaRanges = new Map();
    this.fvaProgress = { running: false, done: 0, total: 0, ms: 0 };
    this.layoutSettings = normalizeLayoutSettings(doc.layoutSettings ?? this.layoutSettings);
    this.view.reactionColorMode = normalizeReactionColorMode(doc.reactionColorMode ?? this.view.reactionColorMode);
    // Older scenario files say nothing about these: they keep what is set.
    for (const key of ['widthByFlux', 'dashNoFlux', 'lineHops']) {
      if (typeof doc[key] === 'boolean') this.view[key] = doc[key];
    }
    this.view.fluxColorStops = fluxColorStops;
    this.view.pathwayRibbonMode = normalizePathwayRibbonMode(doc.pathwayRibbonMode ?? this.view.pathwayRibbonMode);
    this.view.pathwayRibbonWidth = clampNumber(doc.pathwayRibbonWidth ?? this.view.pathwayRibbonWidth, 20, 120, 56);
    this.view.pathwayRibbonOpacity = clampNumber(doc.pathwayRibbonOpacity ?? this.view.pathwayRibbonOpacity, 0.04, 0.45, 0.18);
    this.medium = doc.medium ?? {};
    this.species = doc.species ?? this.species;
    await this.rebuild({ relayout: true, contents: this.scenario?.published === 'steadycom-gut-nine' ? 'none' : 'all', solve: !doc.steadyCom });
    this.restorePathwayManagerState(doc.pathwayManager);
    this.restoreOmicsState(doc.omics);
    this.overrides = new Map((doc.overrides ?? []).map((o) => [o.id, { lower: o.lower, upper: o.upper }]));
    this.applyOverrides();
    // Before solving, so the repaint that follows already reads "Saved".
    this.markScenarioSaved();
    if (doc.steadyCom) await this.solveSteadyComAt(doc.steadyCom.mu);
    else this.solve();
  }
}

/** Whether every community-level bound and extra constraint admits zero flux. steadyComRange caps the total biomass only then. */
function steadyComCapIsSafe(spec) {
  for (const bounds of Object.values(spec.communityExchangeBounds ?? {})) {
    if (bounds.lower > 0 || bounds.upper < 0) return false;
  }
  if (spec.reactions) {
    for (const reaction of spec.reactions.values()) {
      if (reaction.species !== (spec.communityTag ?? 'comm')) continue;
      if (reaction.lowerBound > 0 || reaction.upperBound < 0) return false;
    }
    for (const constraint of spec.constraints ?? []) {
      if ((constraint.lo ?? -Infinity) > 0 || (constraint.hi ?? Infinity) < 0) return false;
    }
  }
  return true;
}

/*
 * A stable number per object, for signatures that must notice an object being
 * replaced (a new omics dataset) without serialising all of it on every paint.
 */
const objectRevisions = new WeakMap();
let nextObjectRevision = 1;
function objectRevision(value) {
  if (!value || typeof value !== 'object') return null;
  if (!objectRevisions.has(value)) objectRevisions.set(value, nextObjectRevision++);
  return objectRevisions.get(value);
}

/**
 * The number a control holds, or null when it holds none.
 *
 * Number('') and Number('  ') are 0, which would turn a cleared bound field
 * into a bound of 0. Empty, blank and non-finite input is "no value" instead.
 */
function finiteInput(value) {
  if (value === null || value === undefined) return null;
  if (typeof value === 'string' && value.trim() === '') return null;
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
}


function uniquePathwayId(name, pathways) {
  const base = String(name)
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[\u0300-\u036f]/g, '')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '') || 'pathway';
  let id = base;
  let index = 2;
  while (pathways.has(id)) {
    id = `${base}-${index}`;
    index += 1;
  }
  return id;
}

function legendFromManagedGroups(groups, colors, reactionIds = null) {
  const allowed = reactionIds == null ? null : new Set(reactionIds);
  const counts = new Map();
  const swatches = new Map();
  for (const [rid, group] of groups) {
    if (allowed && !allowed.has(rid)) continue;
    if (!group || /^Sin pathway$/i.test(group)) continue;
    counts.set(group, (counts.get(group) || 0) + 1);
    if (!swatches.has(group) && colors.has(rid)) swatches.set(group, colors.get(rid));
  }
  return [...counts]
    .map(([label, count]) => ({ label, count, color: swatches.get(label) ?? '#64748b' }))
    .sort((a, b) => a.label.localeCompare(b.label));
}

function clampNumber(value, min, max, fallback) {
  const n = Number(value);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(min, n));
}

/** Whether a parsed map has the drawing a map is made of, whatever it was drawn over. */
function hasMapStructure(doc) {
  return Boolean(doc && Array.isArray(doc.reactions) && Array.isArray(doc.nodes) && Array.isArray(doc.segments));
}

/**
 * The model a map was drawn over, as the user knows it: an imported model by
 * the name of its file, anything else by its species.
 */
function mapModelLabel(parsed) {
  const uploads = (parsed?.metadata?.species ?? [])
    .map((sp) => /^upload:(.+)$/.exec(String(sp.model ?? ''))?.[1])
    .filter(Boolean);
  if (uploads.length) return uploads.map((stem) => `“${stem}”`).join(', ');
  const species = mapSpeciesIds(parsed);
  if (species.length) return `species ${species.map((id) => `“${id}”`).join(', ')}`;
  return 'its model';
}

/** Throw unless a map document can be applied to the community: right structure, known cells, and some reaction (or metabolite) in common. */
function validateMapDocument(doc, community, species) {
  if (!doc || typeof doc !== 'object') throw new Error('The file does not contain a valid map.');
  if (!Array.isArray(doc.reactions) || !Array.isArray(doc.nodes) || !Array.isArray(doc.segments)) {
    throw new Error('The file does not have the expected EditableMap structure.');
  }

  // Every cell the map draws has to be a species of the community. It is
  // checked against the community and not against what happens to be drawn:
  // a map is allowed to draw fewer cells than there are species, and the one on
  // screen may be a blank canvas with none at all, which would turn away every
  // map offered to it. A switched-off species counts: a hand-built map keeps
  // its cell, dimmed, so a map saved that way has to open again.
  const known = new Set((species ?? []).map((sp) => sp.id));
  if ((doc.cells ?? []).some((cell) => !known.has(cell.id))) {
    throw new Error('The map was created for a different set of cells/species.');
  }

  // A model that is still being built loses and renames things between one
  // export and the next, and a map drawn over the earlier one is still a map of
  // this model: what is gone is taken off it when it is applied. Only a map that
  // shares nothing with the active model is a map of something else.
  const reactions = doc.reactions.filter((reaction) => reaction?.id);
  const missing = reactions.find((reaction) => !community.merged.reactions.has(reaction.id));
  if (missing && !reactions.some((reaction) => community.merged.reactions.has(reaction.id))) {
    throw new Error(reactions.length === 1
      ? `Reaction ${missing.id} does not exist in the active model.`
      : `None of the ${reactions.length} reactions on this map exist in the active model (the first is ${missing.id}).`);
  }
  if (!reactions.length) {
    const nodes = doc.nodes.filter((node) => node?.mergedId);
    const lost = nodes.find((node) => !community.merged.metabolites.has(node.mergedId));
    if (lost && !nodes.some((node) => community.merged.metabolites.has(node.mergedId))) {
      throw new Error(`Metabolite ${lost.mergedId} does not exist in the active model.`);
    }
  }
}

function normalizePfbaPercent(value) {
  const n = Number(value);
  if (!Number.isFinite(n)) return 100;
  return Math.min(100, Math.max(0, n));
}

function normalizeSearch(value) {
  return String(value ?? '')
    .normalize('NFD')
    .replace(/[\u0300-\u036f]/g, '')
    .trim()
    .toLowerCase();
}

function describeStatus(status) {
  if (status === 'infeasible') return 'No solution: the constraints are incompatible.';
  if (status === 'unbounded') return 'Unbounded objective: a constraint is missing.';
  if (status === 'iteration_limit') return 'The iteration limit was reached.';
  return status;
}

function pointInsideRect(point, rect, padding = 0) {
  return point.x > rect.x - padding
    && point.x < rect.x + rect.w + padding
    && point.y > rect.y - padding
    && point.y < rect.y + rect.h + padding;
}

function yeastPreviewAnchors(map, speciesId, compartmentId, count) {
  const cell = map?.cells?.find((item) => item.id === speciesId);
  const compartment = cell?.compartments?.find((item) => item.id === compartmentId);
  if (!cell || !compartment || count <= 0) return [];

  const rect = compartment.rect;
  const children = cell.compartments
    .filter((item) => item.parentId === compartmentId)
    .map((item) => item.rect);

  const candidates = [];
  const cols = 8;
  const rows = 6;
  for (let row = 0; row < rows; row += 1) {
    for (let col = 0; col < cols; col += 1) {
      const point = {
        x: rect.x + rect.w * ((col + 1) / (cols + 1)),
        y: rect.y + rect.h * ((row + 1) / (rows + 1)),
      };
      const childPadding = Math.min(rect.w, rect.h) * 0.018;
      if (!children.some((child) => pointInsideRect(point, child, childPadding))) {
        candidates.push(point);
      }
    }
  }

  const pool = candidates.length >= count ? candidates : Array.from({ length: count }, (_, index) => ({
    x: rect.x + rect.w * (((index % 4) + 1) / 5),
    y: rect.y + rect.h * ((Math.floor(index / 4) + 1) / (Math.ceil(count / 4) + 1)),
  }));

  if (count === 1) return [pool[Math.floor(pool.length / 2)]];
  return Array.from({ length: count }, (_, index) => {
    const slot = Math.round(index * (pool.length - 1) / (count - 1));
    return pool[slot];
  });
}

const csv = (s) => `"${String(s).replace(/"/g, '""')}"`;
const frame = () => new Promise((resolve) => {
  if (typeof requestAnimationFrame === 'function') requestAnimationFrame(() => resolve());
  else setTimeout(resolve, 0);
});

/**
 * Wait until what was just drawn is on the screen: past the next frame, and a
 * task after it, which is when the browser has painted. A hidden page gets no
 * frames, so a short timeout stands in for one.
 */
const paintFrame = () => new Promise((resolve) => {
  let done = false;
  const go = () => { if (!done) { done = true; resolve(); } };
  if (typeof requestAnimationFrame === 'function') requestAnimationFrame(() => setTimeout(go, 0));
  setTimeout(go, 60);
});

/** The analyses runAnalysis() knows, by the name the Simulate panel shows. */
const ANALYSIS_LABELS = { fba: 'FBA', pfba: 'pFBA', fva: 'FVA', loopless: 'Loopless FBA' };

/** `850 ms`, `4.2 s`, `2 min 05 s`. */
function formatDuration(ms) {
  if (!Number.isFinite(ms) || ms < 0) return '—';
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)} s`;
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, '0')} s`;
}

/**
 * Which community keeps its full member models and draws a curated map beside
 * them, rather than drawing the members themselves.
 *
 * Only the four-E. coli benchmark does: it has a hand-made central-metabolism
 * overview, so the member models can stay genome-scale without anything trying
 * to draw them.
 */
function usesCuratedCoreMap(communityId) {
  return communityId === 'steadycom-ecoli-auxotrophs';
}

function publishedMemberFileForRuntime(communityId, member) {
  const full = member?.model ?? null;
  const preview = member?.preview ?? null;
  const embedded = globalThis.__ESCHER_MCMC_DATA__;

  /*
   * Without a curated map, whatever is loaded here *is* what gets drawn, and
   * selecting the nine-species community that way takes 557 seconds: nine
   * genome-scale members, ten thousand reactions, laid out as a drawing nobody
   * can read. The excerpt exists for exactly that, and the card already says
   * the growth rate on such a map is the excerpt's. The full model remains what
   * the Download and the SBML conversion read.
   */
  if (!usesCuratedCoreMap(communityId) && communityId !== 'steadycom-gut-nine' && preview) return preview;

  // Hosted/dev builds can read the full member files from models/published/.
  // The standalone intentionally embeds only the much smaller map previews.
  if (embedded) {
    const fullKey = full ? `models/published/${communityId}/${full}` : null;
    const previewKey = preview ? `models/published/${communityId}/${preview}` : null;
    if (fullKey && fullKey in embedded) return full;
    if (previewKey && previewKey in embedded) return preview;
  }

  return full ?? preview;
}

/** Which published communities are distributed as SteadyCom problems. */
const STEADYCOM_COMMUNITIES = new Set(['steadycom-ecoli-auxotrophs', 'steadycom-gut-nine']);

function steadyComCrossFeeding(spec, fluxes, threshold = 1e-6) {
  const links = [], perMetabolite = [];
  let exchanges;
  if (spec.reactions) exchanges = communityBalances(spec, fluxes);
  else {
    const speciesExchanges = new Map();
    for (const member of spec.members) for (const [base, rid] of Object.entries(member.exchangeFor)) {
      if (!speciesExchanges.has(base)) speciesExchanges.set(base, new Map());
      speciesExchanges.get(base).set(member.id, fluxes.get(`${member.id}|${rid}`) ?? 0);
    }
    exchanges = { speciesExchanges, communityUptake: new Map(), communityExport: new Map() };
    for (const [base, bySpecies] of speciesExchanges) {
      const net = [...bySpecies.values()].reduce((sum, value) => sum + value, 0);
      exchanges.communityUptake.set(base, Math.max(0, -net));
      exchanges.communityExport.set(base, Math.max(0, net));
    }
  }
  for (const [mid, bySpecies] of exchanges.speciesExchanges) {
    const base = spec.metadata?.bases?.get(mid) ?? mid;
    const producers = [], consumers = [];
    for (const [speciesId, value] of bySpecies) {
      if (value > threshold) producers.push({ speciesId, flux: value });
      if (value < -threshold) consumers.push({ speciesId, flux: -value });
    }
    const uptake = exchanges.communityUptake.get(mid) ?? 0;
    const secretion = exchanges.communityExport.get(mid) ?? 0;
    if (uptake > threshold) producers.push({ speciesId: null, flux: uptake });
    if (secretion > threshold) consumers.push({ speciesId: null, flux: secretion });
    const output = producers.reduce((sum, item) => sum + item.flux, 0);
    const input = consumers.reduce((sum, item) => sum + item.flux, 0);
    const total = Math.min(output, input);
    if (output > 0 && input > 0) for (const p of producers) for (const q of consumers) {
      const flux = (p.flux / output) * (q.flux / input) * total;
      if (flux > threshold) links.push({ base, from: p.speciesId, to: q.speciesId, flux });
    }
    if (producers.length || consumers.length) perMetabolite.push({ base, producers, consumers, total });
  }
  links.sort((a, b) => b.flux - a.flux);
  return { links, perMetabolite };
}

function staleSteadyComError() {
  const error = new Error('The community constraints changed; this calculation was discarded.');
  error.name = 'AbortError';
  return error;
}

async function fetchText(url) {
  const embedded = globalThis.__ESCHER_MCMC_TEXT__;
  if (embedded) {
    const key = String(url).replace(/^.*?(?=models\/)/, '');
    if (key in embedded) return embedded[key];
  }
  const res = await fetch(url);
  if (!res.ok) throw new Error(`Could not load ${url} (${res.status})`);
  return res.text();
}

async function fetchJSON(url) {
  // The single-file build embeds the catalogue, because fetch() is blocked on
  // file:// and the whole point of that build is to open without a server.
  const embedded = globalThis.__ESCHER_MCMC_DATA__;
  if (embedded) {
    const at = url.indexOf('models/');
    const key = at >= 0 ? url.slice(at) : url;
    if (key in embedded) return JSON.parse(JSON.stringify(embedded[key]));
  }
  const res = await fetch(url);
  if (!res.ok) throw new Error(`Could not load ${url} (${res.status})`);
  return res.json();
}

export { OBJECTIVES };
