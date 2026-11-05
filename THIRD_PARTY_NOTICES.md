# Third-party notices

The root MIT licence applies to Coxeter-MFV's own source. It does not relicense third-party software, models or data. Distribute the applicable notices and source with application releases.

## Browser distribution

`app/web/coxeter_mfv.html` embeds glpk.js 5.0.0 and GNU GLPK. The combined browser distribution is provided under GPL-3.0, while Coxeter-MFV's own source remains available under MIT. The full GPL text is in [`LICENSES/GPL-3.0.txt`](LICENSES/GPL-3.0.txt) and is embedded in new standalone builds.

The vendored factory patch and file provenance are documented in [`code/assets/vendor/PROVENANCE.md`](code/assets/vendor/PROVENANCE.md). Matching glpk.js and GLPK source archives, exact revisions, checksums and build instructions are preserved in [`code/assets/vendor/source`](code/assets/vendor/source/README.md). The wrapper and standalone build scripts are part of this repository's source.

The npm dependency graph, including pako's MIT/Zlib terms where applicable, is recorded in `code/package-lock.json`. Installing the npm packages is a build/test step; their presence in that graph does not mean that every package is embedded in the browser artifact. The embedded solver uses the direct glpk.js factory and WASM, rather than the npm worker loader.

## Windows distribution

The native application includes the HiGHS solver. Its MIT licence is preserved in [`LICENSES/HiGHS-MIT.txt`](LICENSES/HiGHS-MIT.txt). The locked Rust dependency graph is in `code/rust/Cargo.lock`. Dependency licence texts and upstream references are collected in [`app/windows/THIRD_PARTY_LICENSES.txt`](app/windows/THIRD_PARTY_LICENSES.txt), including build and target-specific dependencies in that graph.

Regenerate native notices with `python tools/native-licenses.py` after changing the lockfile, and review any missing upstream licence text. Include the resulting notices when distributing the executable independently of the repository.

Gurobi and CPLEX are optional integrations loaded from a user's own installation. Their software and licences are not distributed by this project. The installed solver's licence and size limits apply.

## Models, maps and data

Dataset provenance, modifications and known terms are documented in [`data/README.md`](data/README.md) and each example README. An article citation is attribution, not a substitute for redistribution permission. Entries whose source terms remain unverified must be resolved before a public publication release.

The Zuñiga MATLAB source models are not redistributed. They are obtained from the authors' pinned repository revision and checked against recorded SHA-256 values. An article's open-access licence should not be assumed to cover separately hosted model files.

## Upstream sources

- [glpk.js](https://github.com/jvail/glpk.js)
- [GNU GLPK](https://www.gnu.org/software/glpk/)
- [HiGHS](https://github.com/ERGO-Code/HiGHS)
- [GNU GPL version 3](https://www.gnu.org/licenses/gpl-3.0.html)
