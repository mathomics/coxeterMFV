# Reproducibility

Use the exact release and source data behind a result. The working repository may continue to change after an official snapshot is promoted.

## Identify the software

`release-manifest.json` records the promoted working revision, a digest of the distributed source, and SHA-256 checksums of both applications. From the repository root:

```bash
node tools/release-manifest.mjs
```

This verifies the local files against the recorded manifest. It does not establish the authenticity of an independently supplied manifest; obtain the repository or release from the official project. A public release should also have a version tag and permanent archive DOI.

## Run the automated checks

Requires Node.js 20 or later. Browser automation uses a global WebSocket implementation and should run with Node.js 22 or later. From the repository root:

```bash
cd code
npm ci
npm test
npm run validate
npm run build:standalone
npm run check:standalone
```

`npm ci` installs the exact versions in `package-lock.json`, including the GLPK backend used by solver tests. Vendored GLPK files are included for browser builds. Some optional integration tests skip when private external source files are absent; inspect the test summary rather than treating a skipped test as a validation result.

The standalone check needs Chrome, Chromium or Edge. Set `CHROME_PATH` to the browser executable if it is not detected. To check the distributed browser file instead of a new build:

```bash
npm run check:standalone -- --file=../app/web/coxeter_mfv.html
```

`npm run check:sbml` checks exported SBML with the optional `python-libsbml` package. The command reports a skip if libSBML is unavailable. Native build instructions are in [`code/README.md`](../code/README.md); use `cargo test --locked --workspace --lib` from `code/rust/` for library tests. Tests that require missing parity fixtures do not establish JavaScript/Rust parity.

## Minimal numerical example

`npm run validate` solves all five bundled teaching models, checks mass and charge balance where elemental formulas are available, and reports reactions that cannot be checked. A successful run reports positive optimal growth and `All 5 models validated`. This is a small executable usage example with explicit success criteria, separate from the genome-scale case studies.

The tests in `code/test/` cover model import/export, solver behaviour, map editing, reproducibility bundles and published-community adapters. They provide validation of implemented behaviour; they do not independently reproduce every result or figure in an unpublished manuscript.

The broader `npm run check:browser` harness needs a running source server (`npm run serve` in another terminal). It has known failures in this snapshot. See [VALIDATION.md](VALIDATION.md) for the actual results, baseline comparison and limitations of the distribution CI.

## Literature examples

| Example | Local inputs | Verification and limits |
| --- | --- | --- |
| Yeast9 | Model and curated map in `examples/yeast9-cytosol-mitochondria-peroxisome/` | Import the model, then the map. A model optimum depends on its bounds and objective; the map is a subset of the model. |
| Stolyar syntrophy | Joint and member models, medium, bounds, map and manifest | Tests in `code/test/stolyar.test.mjs` and `published-communities.test.mjs` check the imported structure and behaviours. |
| SteadyCom auxotrophs | Four member models, medium, bounds, map and manifest | The example documents a maximum growth of approximately 0.73599 h⁻¹. Preserve the original community medium and knockout settings when comparing. |
| SteadyCom gut community | Nine full member models, medium, bounds and map | The drawing is an excerpt. Full-model reconstruction checks are in `steadycom-joint-adapter.test.mjs`; a drawing is not evidence of a solved full-community result. |
| Zuñiga phenylalanine rescue | Pinned-source manifest, saved results and map | Full source models are downloaded separately. Saved predictions and the numerical comparison criterion are documented in the example README. |

Do not regenerate published model assets during a routine software build. Generation tools may need upstream spreadsheets, MATLAB files or external runtimes that are not distributed here. The source licence and access conditions of each input are part of its provenance.

## Record a new analysis

Save the application version and source revision, input hashes, model source and licence, medium and bounds, objective and direction, gene knockouts, community composition and abundances, method and parameters, solver and version, numerical tolerances and termination status. Keep the scenario, model, map, flux table and reproducibility bundle together.

Check both steady-state residuals and bound violations before interpreting a result. Report differences between alternative optima, numerical tolerance, incomplete searches and infeasible problems explicitly. For FVA or a community-growth search, retain the requested objective fraction, total-flux factor or verified search interval as appropriate.

The final manuscript must identify which release, scripts, parameters, inputs and outputs reproduce each reported figure and table. Those manuscript-specific materials must be curated and archived before submission; the [publication checklist](PUBLICATION.md) tracks this separately from the general application tests.
