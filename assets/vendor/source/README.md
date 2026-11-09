# GLPK corresponding source

The browser distribution includes `glpk.js@5.0.0` and its GNU GLPK WebAssembly binary. These archives preserve the matching upstream source, including the submodule omitted by GitHub's automatic source archive.

| Archive | Upstream revision | SHA-256 |
| --- | --- | --- |
| `glpk.js-5.0.0.tar.gz` | `80e1ff5bd80b588bc233a4427b0687128146c8d0`, the `gitHead` of npm `glpk.js@5.0.0` | `c9181a09422bc2e46283f1563f4cb06c9a6ce0ceca27eaecd7e84ffa1979fecf` |
| `glpk-4945cf1.tar.gz` | GLPK submodule `4945cf1cb8e860b24d8b79a8a258c2d214945d10` | `a667028edb376ffb6d9a862174247957430307e76aa55534c551b0530953c862` |

To rebuild upstream, extract the first archive, place the contents of the second in its `src/glpk/` directory, and follow the archived README, Makefile, Dockerfile and package scripts. The upstream build uses Emscripten 4.0.3. Preserve the locked npm dependencies when rebuilding. The source and build instructions for Coxeter's wrapper patch are in `../PROVENANCE.md` and `../../../tools/vendor-glpk.mjs`.

The packaged WASM was checked byte-for-byte against the npm 5.0.0 distribution. Coxeter's vendored JavaScript forwards the factory arguments as documented in the provenance file. The standalone builder embeds those bytes and changes module loading for the single-file application.

The full GPL version 3 text is in `../LICENSE` and at the repository root in `LICENSES/GPL-3.0.txt`. Distribute the corresponding source, project source and licences alongside GPL-containing application downloads.

- [glpk.js source revision](https://github.com/jvail/glpk.js/tree/80e1ff5bd80b588bc233a4427b0687128146c8d0)
- [GLPK source revision](https://salsa.debian.org/science-team/glpk/-/tree/4945cf1cb8e860b24d8b79a8a258c2d214945d10)
