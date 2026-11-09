# Validation record

Validation of the official pre-publication snapshot promoted on 6 November 2026 from working commit `79d9167656a4457809fd92098a5b46481dedbd5a`. Checks marked *4 November* were run on the previous snapshot, promoted from `bf4bcfe5ec13ea04504c7428eaf5e63822c23c8f`, and have not been repeated. The exact distributed application bytes and source digest are recorded in [`release-manifest.json`](../release-manifest.json).

The official repository adds release packaging, documentation, licence material and native version configuration around that source. The working repository's tracked files were left unchanged.

## Checks performed

| Check | Result and scope |
| --- | --- |
| `npm test`, after `npm ci` | 1192 tests: 1178 passed, 14 skipped, 0 failed. Optional source-dependent tests remain skipped. |
| `npm run validate` | All five bundled teaching models passed the solver and balance checks. Reactions involving photons without elemental formulas were reported as unchecked. |
| `cargo test --locked --workspace --lib` | 471 library tests passed across nine crates. Some parity-test helpers return early when external fixtures are absent; a passing count does not establish complete cross-language parity. |
| Standalone build and `check:standalone` | All 11 checks passed in Chromium 156. The packaged HTML started from a local file, used embedded GLPK, displayed the published communities, offered complete model downloads and retained the selected community on return to the map. |
| Chromium and Firefox smoke checks | *4 November.* Both engines started the packaged HTML, solved the bundled consortium to objective 0.1578466395282152 with GLPK, and opened the four-strain SteadyCom example with its reference value and 1112 drawn nodes. This is a bounded smoke check, not exhaustive browser compatibility testing. |
| Native release build | Built from the promoted source for Windows x64 with embedded models, a static MSVC C runtime and release provenance variables. |
| Native `--check-solvers` | All HiGHS 1.15.0 self-checks passed, including LP statuses, duals, warm start, interruption, MILP, FBA, pFBA, FVA and loopless FBA. Commercial solvers were unavailable for testing. |
| Native rendering | A 1440 × 900 offscreen screenshot of the bundled four-species map rendered successfully and was visually inspected. |
| `npm run check:sbml` | *4 November.* 20 exported SBML files checked with libSBML, 0 files with errors. The validator reported 48414 warnings, including units and initial-quantity warnings; this is not a warning-free standards report. |
| Citation metadata | *4 November.* `CITATION.cff` validated against the official CFF 1.2.0 JSON schema. |
| Native dependency notices | *4 November*, with the same `Cargo.lock`. Licence texts collected for 288 packages in the locked dependency graph, with pinned upstream supplements for packages whose registry archives omit licence texts. |

The local environment used Windows, Node.js 24.14.1, Rust 1.99.0, Chromium 156.0.8078.4 and Firefox 157.0. These browser builds were installed for validation and do not establish compatibility with all production browser versions. Linux, macOS, Windows 10 and mobile devices were not independently tested in this preparation.

## Broader browser integration failures

*4 November, previous snapshot.* `npm run check:browser` completed with **14 failed checks** on the promoted source. All 14 named checks also failed when the same harness was run against the clean working revision; that run had 17 failures in total, with three additional community-card checks. This comparison demonstrates pre-existing failures. It does not establish whether each failure is an application defect, a stale assertion or a timing issue.

| Area | Failed expectations in the promoted-source run |
| --- | --- |
| Analysis dispatch | Switching through the harness's pFBA and FBA Run Analysis paths, two checks. |
| Map interaction | Camera position during node dragging and metabolite-label tracking, two checks. |
| Interface structure | Expected cell/compartment counts and compartment-inspector instructions, two checks. |
| Model/map separation | Unchanged optima after clearing or partially drawing a map; the harness compared non-numeric values, two checks. |
| Published community loading | Expected full-member versus preview-model detection, one check. |
| Theme state | Expected dark and light state after toggling, two checks. |
| Map restoration | Rejection of a missing scenario context, one check. |
| SteadyCom interface | Expected growth-range and composition wording, one check. |
| Yeast9 scenario | Expected an already solved result after loading, one check. |

Investigate and resolve these in the working repository, update the corresponding tests where warranted, and promote the verified fix before declaring the publication release ready. The distribution CI runs the unit/model checks and standalone smoke checks; it does not certify that this broader harness passes.

## Checks still requiring another environment or publication decisions

Docker and Podman were unavailable on this host, so the container recipe was not built locally. The GitHub workflow includes a container build and a byte comparison of the served HTML, but that workflow has not been executed remotely as part of this preparation.

The publication archive DOI, confirmed software authors, public maintainer contact, dataset redistribution review and manuscript figure-by-figure reproduction record remain separate release requirements. They are tracked in [PUBLICATION.md](PUBLICATION.md).
