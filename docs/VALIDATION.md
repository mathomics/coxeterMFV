# Validation record

Validation of the pre-publication snapshot promoted on 18 November 2026 from the working repository at `bd2ffb52e8be65e17a195a0f8a86d2db89f780e9` and its uncommitted changes. Rows marked *4 November* were run on an earlier snapshot and have not been repeated. The exact distributed application bytes and source digest are recorded in [`release-manifest.json`](../release-manifest.json).

The official repository adds release packaging, documentation, licence material and native version configuration around that source.

## Checks performed

| Check | Result and scope |
| --- | --- |
| `npm test` | 1221 tests, 1207 passed, 14 skipped and 0 failed. Optional source-dependent tests remain skipped. |
| `npm run validate` | All five bundled teaching models passed the solver and balance checks. Reactions involving photons without elemental formulas were reported as unchecked. |
| `cargo test --locked --workspace --lib` | 495 library tests passed across nine crates, 0 failed. |
| `npm run check:browser` | All 207 checks of the source-browser integration harness passed in Chromium 156, on the source served by `npm run serve`. |
| Standalone build and `check:standalone` | All 11 checks passed in Chromium 156. The packaged HTML started from a local file, solved with the embedded GLPK, showed the published communities, offered complete model downloads and kept the selected community when returning to the map. |
| Offline use and phone width | With networking switched off, the packaged HTML opened from a local file, solved the *E. coli* example with pFBA and the four-strain community with SteadyCom, and made no network requests. At a 390 px phone width the page did not scroll sideways. The Yeast9 and phenylalanine rescue examples download their models from the original sources and need a connection. |
| Firefox | Firefox 160 opened the packaged HTML from a local file, solved the bundled consortium, ran pFBA and FVA on *E. coli* and reproduced the four-strain SteadyCom μmax of 0.736 after 20 LPs. |
| Case studies | Yeast9 grew at 0.081 h⁻¹ with total absolute fluxes of 387104 (FBA), 100.8 (pFBA) and 107.8 (CycleFreeFlux). In the phenylalanine rescue the wild type grew at 0.088 h⁻¹, the coculture at 0.078 and 0.035 h⁻¹, and 0.014 mmol gDW⁻¹ h⁻¹ of phenylalanine passed from *S. elongatus* to *E. coli*. The four auxotrophic strains reached μmax 0.736 h⁻¹ after 20 LPs. |
| Model import and export | `e_coli_core` in SBML, COBRA JSON and MAT, a COBRA Toolbox Excel model, CarveMe-style compartment identifiers, iJO1366 and yeast-GEM 9.1.1 were imported and solved. A broken file was refused with a message and the open model kept. SVG, PNG, CSV, model JSON, SBML, merged model JSON, map JSON and a reproducibility bundle were exported. The SBML reopened with the same growth and the bundle replayed with zero flux difference. |
| Website and container command | The site assembled as `.github/workflows/pages.yml` does opened every example link of its home page and solved the bundled examples. The packaged HTML served by `python -m http.server`, the command the `Dockerfile` runs, did the same. |
| Native release build | Built from the promoted source for Windows x64 with embedded models, a static MSVC C runtime and release provenance variables. |
| Native `--check-solvers` | All HiGHS 1.15.0 self-checks passed, including LP statuses, duals, warm start, interruption, MILP, FBA, pFBA, FVA and loopless FBA. Gurobi and CPLEX were unavailable for testing. |
| Native rendering | Offscreen 1400 × 900 screenshots of the bundled communities rendered and were inspected. |
| `npm run check:sbml` | *4 November.* 20 exported SBML files checked with libSBML, 0 files with errors. The validator reported 48414 warnings, including units and initial-quantity warnings, so this is not a warning-free standards report. |
| Citation metadata | *4 November.* `CITATION.cff` validated against the official CFF 1.2.0 JSON schema. |
| Native dependency notices | *4 November*, with the same `Cargo.lock`. Licence texts collected for 288 packages in the locked dependency graph, with pinned upstream supplements for packages whose registry archives omit licence texts. |

The local environment used Windows 11, Node.js 24.14.1, Rust 1.99.0, Chromium 156.0.8078.4 and Firefox 160.0.1. These browser builds were installed for validation and do not establish compatibility with all production browser versions. Linux, macOS, Windows 10 and physical mobile devices were not independently tested in this preparation.

## Checks that need another environment

Docker and Podman were unavailable on this host, so the container image was not built locally. The packaged HTML was served with the same `python -m http.server` command the image runs. The GitHub workflow builds the image and compares the served HTML byte for byte.
