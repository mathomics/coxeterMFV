# Building from source

The repository root holds the complete source of Coxeter-MFV. The JavaScript web application is the reference implementation. The Rust desktop application in [`rust`](../rust) is a native port that reads and writes the same files and is tested against the outputs of the JavaScript modules.

## Layout

| Folder | Contents |
| --- | --- |
| [`src`](../src) | Web application. Models and communities, map layout and editing, rendering, solvers, omics and the interface |
| [`assets`](../assets) | Stylesheets and the vendored GLPK solver |
| [`models`](../models) | Bundled teaching models, scenarios and published communities |
| [`test`](../test) | Test suite of the web application |
| [`tools`](../tools) | Build, validation, release and model-generation scripts |
| [`rust`](../rust) | Desktop application, one crate per part of the program |
| [`docs/gut9-map`](gut9-map) | Reference maps of the nine-species gut community used by the tests |

## Web application

Requires [Node.js](https://nodejs.org) 20 or later. Use Node.js 22 or later for the browser automation commands. Install the locked npm dependencies before running solver tests; the browser's GLPK files are already vendored. From the repository root:

```bash
npm ci
npm test
npm run validate
npm run serve
npm run build:standalone
```

`npm run build:standalone` writes `coxeter_mfv.html` at the repository root; the distributed copy is [`app/coxeter_mfv.html`](../app/coxeter_mfv.html).

## Desktop application

Requires [Rust](https://rustup.rs) 1.95 or later for the locked GUI dependencies; this snapshot was validated with Rust 1.99.0. The HiGHS solver is compiled from source, which needs CMake, a C++ compiler and libclang.

```bash
cd rust
cargo run --release -p coxeter-app      # build and start the application
```

On Windows, the Microsoft C++ Build Tools provide the compiler, and CMake and libclang can be installed with `pip install cmake libclang`. Point the `LIBCLANG_PATH` environment variable at the `clang/native` folder of that package.

The release build compiles the bundled models into the program. External model downloads remain separate. Use `cargo build --locked --release -p coxeter-app` for the desktop build, and `cargo test --locked --workspace --lib` for the library tests. Missing optional parity fixtures limit what those tests verify.

The crates and their relation to the web application are described in [`rust/README.md`](../rust/README.md). Official promotion and packaging instructions are in [RELEASING.md](RELEASING.md); usage and numerical validation are covered in the [user guide](USER_GUIDE.md) and [reproducibility guide](REPRODUCIBILITY.md).
