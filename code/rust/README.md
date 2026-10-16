# Coxeter-MFV in Rust

A native rewrite of Coxeter Multicell Flux Visualizer: one program for Linux,
Windows and macOS, drawn on the GPU, with a native multicore solver (HiGHS).
It reads and writes the same files as the JavaScript app (maps, models,
scenarios), so a map can move between the two.

The JavaScript app in `../src` is the specification. Every crate here is tested
against outputs of the JavaScript modules themselves (see *Testing*).

## Build and run

Needs Rust (`rustup`, stable) and, for the HiGHS solver, `cmake` and a C++
compiler.

```bash
cd rust
cargo run --release -p coxeter-app      # the application
cargo test                              # every test
```

## Layout

| Crate | What it does | Ported from |
|---|---|---|
| `coxeter-core` | Models, communities, compartments, labels, map data, geometry, JS-compatible numerics | `src/core/*`, `src/layout/geometry.js`, `src/map/boundaries.js`, `src/map/metabolite-label.js` |
| `coxeter-io` | SBML / JSON / MAT / XLSX in and out, model editing, provenance, reproducibility | `src/core/sbml*.js`, `model-import.js`, `model-edit.js`, `provenance.js`, `reproducibility.js`, `xlsx.js`, `src/import/mat5.js` |
| `coxeter-solver` | FBA, pFBA, loopless FBA, parallel FVA, duals, SteadyCom on HiGHS | `src/solver/*`, `src/core/steadycom.js`, `src/communities/steadycom-*.js` |
| `coxeter-map` | Automatic layout, reaction placement, map editing, history, reconciliation, map documents | `src/layout/*`, `src/map/*` |
| `coxeter-render` | The map's SVG, styles, flux colours, cross-feeding routes; SVG/PNG export | `src/render/*` |
| `coxeter-analysis` | Published communities, omics overlays, GIMME, cross-feeding explorer | `src/communities/*`, `src/omics/*`, `src/analysis/*` |
| `coxeter-app` | The application: state and actions, and the native interface | `src/ui/*` |

## Testing

`tools/fixtures-<area>.mjs` run the JavaScript modules on the bundled models,
scenarios and maps and write their outputs to `fixtures/<area>/`. The Rust
tests read those files and require the same answer — exactly, where the
computation is deterministic. To regenerate after changing the JS app:

```bash
for f in rust/tools/fixtures-*.mjs; do node "$f"; done   # from the repo root
```

Genome-scale references are written to `fixtures/large/` (not committed);
the tests that need them skip when they are absent.

