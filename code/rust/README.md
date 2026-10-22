# Coxeter-MFV in Rust

A native rewrite of Coxeter Multicell Flux Visualizer: one program for Linux,
Windows and macOS, drawn on the GPU, with a native multicore solver (HiGHS,
or the Gurobi or CPLEX installed on the computer; see *Solvers*).
It reads and writes the same files as the JavaScript app (maps, models,
scenarios), so a map can move between the two.

The JavaScript app in `../src` is the specification. Every crate here is tested
against outputs of the JavaScript modules themselves (see *Testing*).

## Prebuilt

`bin/coxeter-mfv.exe` is a release build for Windows x64 with the bundled
models compiled in: it runs on its own, no Rust needed. The coxeterMFV
repository publishes the same file as `app/windows/coxeter-mfv.exe` (its copy
of this source leaves `bin/` out).

To rebuild it, `cargo build --release -p coxeter-app` on Windows writes
`target/release/coxeter.exe`. On Linux the same Windows build is made with
MinGW-w64 (`sudo apt install mingw-w64` once), and lands in
`target/x86_64-pc-windows-gnu/release/coxeter.exe`:

```bash
rust/tools/build-windows-exe.sh
```

Either way, copy it to `bin/coxeter-mfv.exe`.

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
| `coxeter-solver` | FBA, pFBA, loopless FBA, parallel FVA, duals, SteadyCom on HiGHS, Gurobi or CPLEX | `src/solver/*`, `src/core/steadycom.js`, `src/communities/steadycom-*.js` |
| `coxeter-map` | Automatic layout, reaction placement, map editing, history, reconciliation, map documents | `src/layout/*`, `src/map/*` |
| `coxeter-render` | The map's SVG, styles, flux colours, cross-feeding routes; SVG/PNG export | `src/render/*` |
| `coxeter-analysis` | Published communities, omics overlays, GIMME, cross-feeding explorer | `src/communities/*`, `src/omics/*`, `src/analysis/*` |
| `coxeter-app` | The application: state and actions, and the native interface | `src/ui/*` |

## Solvers

HiGHS is built in and is the default. Gurobi (9.0 or later) and CPLEX (12.6 or
later) are used from the computer's own installation, with its own licence:
nothing of them is linked into the program or shipped with it. Their library
is looked for where the installers put it (`GUROBI_HOME`,
`CPLEX_STUDIO_BINARIES*` and `CPLEX_STUDIO_DIR*`, the search path, then
`C:\gurobi*`, `C:\Program Files\IBM\ILOG\CPLEX_Studio*`, `/opt/gurobi*`,
`/opt/ibm/ILOG/CPLEX_Studio*`, `/Library/gurobi*`, `/Applications/CPLEX_Studio*`)
and opened at run time. *Simulate → Simulation settings → Solver* chooses one,
and the choice is remembered; *Locate Gurobi…* and *Locate CPLEX…* point at a
library installed elsewhere. Opening a library and checking its licence runs
in the background, the first time the page is shown.

Every analysis runs on the chosen solver (FBA, pFBA, FVA, loopless FBA,
SteadyCom, GIMME), and the results, the methods text and reproducibility
bundles name it with its version; a bundle replayed on another solver says so.
Where the optimum is not unique, two solvers can return different fluxes for
the same objective value. Restricted licences are told apart: CPLEX Community
Edition (1000 variables and 1000 constraints) and the licence that comes with
`pip install gurobipy` (2000 each) refuse larger problems, and the message
says why.

```bash
coxeter --solvers          # the solvers found on this computer
coxeter --check-solvers    # a battery of problems with known answers on each
coxeter --solver gurobi    # start on Gurobi (or highs, cplex)
```

The tests run Gurobi and CPLEX where they are installed with a licence:
`coxeter-solver/tests/backends.rs` against the JS fixtures and
`coxeter-state/tests/solver_choice.rs` through the Store. Where they are not,
those tests say so and pass.

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

## Working on it

`AGENTS.md` holds the rules every part of the port follows: which JavaScript
behaviours need the helpers in `coxeter_core::js`, how points shared with a
reaction's axis are handled, and how parity is tested.
