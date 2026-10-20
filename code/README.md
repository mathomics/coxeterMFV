# Source code

This folder holds the complete source of Coxeter-MFV. The JavaScript web application is the reference implementation. The Rust desktop application in [`rust`](rust) is a native port that reads and writes the same files and is tested against the outputs of the JavaScript modules.

## Layout

| Folder | Contents |
| --- | --- |
| [`src`](src) | Web application. Models and communities, map layout and editing, rendering, solvers, omics and the interface |
| [`assets`](assets) | Stylesheets and the vendored GLPK solver |
| [`models`](models) | Bundled teaching models, scenarios and published communities |
| [`test`](test) | Test suite of the web application |
| [`tools`](tools) | Build, validation and model-generation scripts |
| [`rust`](rust) | Desktop application, one crate per part of the program |
| [`docs/gut9-map`](docs/gut9-map) | Reference maps of the nine-species gut community used by the tests |

## Web application

Requires [Node.js](https://nodejs.org) 20 or later. There are no package dependencies to install.

```bash
cd code
npm test                      # run the test suite
npm run serve                 # serve index.html locally
npm run build:standalone      # write the single-file coxeter_mfv.html
```

## Desktop application

Requires [Rust](https://rustup.rs) 1.85 or later. The HiGHS solver is compiled from source, which needs CMake, a C++ compiler and libclang.

```bash
cd code/rust
cargo run --release -p coxeter-app      # build and start the application
cargo test                              # run every test
```

On Windows, the Microsoft C++ Build Tools provide the compiler, and CMake and libclang can be installed with `pip install cmake libclang`. Point the `LIBCLANG_PATH` environment variable at the `clang/native` folder of that package.

The release build compiles the bundled models into the program. The crates and what each one ports from the web application are described in [`rust/README.md`](rust/README.md).
