# Preparing an official release

This repository contains reviewed distribution snapshots. Develop application changes in EscherMultiCellMultiCompartment, then promote a known revision here. Preserve earlier tagged releases so published analyses remain reproducible.

## Promote the source

1. Choose a clean, committed working revision and record its full commit identifier. Review application changes and any new data or dependencies before copying.
2. Copy the tracked application inputs into `code/`: `src/`, required `assets/`, `models/`, `test/`, `tools/`, `index.html`, `package.json`, `package-lock.json`, Rust workspace manifests, crate source and required build configuration. Keep the reference maps used by tests. Exclude `node_modules/`, `target/`, caches, private models, unrelated user inputs, manuscript drafts and development logs.
3. Refresh the public examples when their corresponding bundled model or map has changed. Verify the source, licence and modifications of each third-party input.
4. Update the software version consistently in `code/package.json`, `code/package-lock.json`, `code/rust/Cargo.toml`, `code/rust/Cargo.lock`, `code/rust/.cargo/config.toml` and `CITATION.cff`. Update `CHANGELOG.md`. A source commit is provenance, not a replacement for a release version.

## Build and verify

From `code/`:

```bash
npm ci
npm test
npm run validate
npm run build:standalone -- ../app/web/coxeter_mfv.html
npm run check:standalone -- --file=../app/web/coxeter_mfv.html
```

Set `COXETER_GIT_COMMIT` to the promoted working revision and `COXETER_GIT_BRANCH` to `release-snapshot` before building, so the browser's embedded build identity names its source. The build includes a timestamp, so byte-for-byte rebuild equality is not expected. The release manifest records the exact distributed bytes.

On Windows, build from `code/rust/` with a supported MSVC toolchain, CMake and libclang:

```powershell
cargo test --locked --workspace --lib
cargo build --locked --release -p coxeter-app
Copy-Item -LiteralPath target/release/coxeter.exe -Destination ../../app/windows/coxeter-mfv.exe
```

The repository's `.cargo/config.toml` selects a static C runtime for MSVC. Set `LIBCLANG_PATH` if bindgen cannot find libclang. A release build embeds the model catalogue. When reusing a shared `CARGO_TARGET_DIR`, copy the executable from that directory instead. The alternative Linux/MinGW cross-build script is in `code/rust/tools/build-windows-exe.sh`.

The native Cargo configuration sets `COXETER_BUILD_VERSION` to the release version so provenance reports agree with the package and executable versions. Set `COXETER_GIT_COMMIT`, `COXETER_GIT_BRANCH` and `COXETER_BUILD_TIME` before the native build as well. The native provenance report reads those compile-time values. After a dependency change, run `python tools/native-licenses.py` from the repository root and review the generated notices.

Run the executable with `--help`, `--solvers` and `--check-solvers`. Open a bundled example, exercise imports and exports, and compare the corresponding browser workflow. Verify the minimum supported operating systems and browser engines on actual hosts. Record skipped tests and absent optional fixtures.

From the repository root:

```bash
docker build -t coxeter-mfv .
docker run --rm -p 127.0.0.1:8080:8080 coxeter-mfv
```

Open the container's page in a browser and run the same minimal example. Record the base image digest and container platform with the release validation record.

After the source and both builds are final:

```bash
node tools/release-manifest.mjs --write --source-commit=FULL_WORKING_COMMIT
node tools/release-manifest.mjs
```

Replace `FULL_WORKING_COMMIT` with the full 40-character identifier. Writing the manifest records hashes; it does not prove that the binaries were built from the source. The build and validation record supplies that evidence. Never regenerate hashes merely to hide an unexplained difference.

## Publish and archive

Complete [the publication checklist](PUBLICATION.md), including the manuscript-specific reproducibility materials and rights review. Create the version tag from the reviewed official commit. Attach the browser HTML, Windows executable, licence notices, corresponding third-party source and `release-manifest.json` to the release. Keep the source archive and original model references accessible alongside the downloads.

Archive that exact release and the manuscript analysis materials in the journal's accepted long-term repository. Obtain the permanent DOI before writing the final availability statement. Confirm current NAR archiving instructions, which specifically name Figshare in the detailed software section.

Add the real archive DOI to `CITATION.cff`; add a `preferred-citation` for the article once its complete reference and DOI are assigned. Complete the release date and changelog without rewriting older entries. Verify the public links, support channels, permissions and download checksums before announcing the release.
