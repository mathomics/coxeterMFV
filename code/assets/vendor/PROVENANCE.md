# assets/vendor

Third-party files, vendored so the browser build loads them without a bundler.
Regenerate with `npm run vendor:glpk`; do not edit them by hand.

## glpk.js 5.0.0

| | |
|---|---|
| Source | npm `glpk.js@5.0.0` |
| Upstream | https://github.com/jvail/glpk.js.git |
| Licence | **GPL-3.0** (GLPK itself is GPL; see `node_modules/glpk.js/LICENSE`) |
| `glpk.js` | Emscripten glue, direct build — sha256 af16e8e1639eec61… (patched, see below) |
| `glpk.wasm` | GLPK compiled to WebAssembly — sha256 acf09a204bbed381… (verbatim) |

### The one patch applied

The published ESM export drops the factory's arguments:

```js
var t=()=>r();export{t as default}
```

which makes it impossible to hand the module a pre-fetched `wasmBinary`. A
browser needs exactly that, because the glue otherwise attempts a synchronous
fetch of the `.wasm` and aborts. The inner factory `r` already accepts the
overrides, so the fix is to forward them:

```js
var t=(...a)=>r(...a);export{t as default}
```

Nothing else is changed, and `glpk.wasm` is copied byte for byte.

### Licensing note

This project's own code is MIT. GLPK is GPL-3.0. It is loaded as an optional
backend: `src/solver/simplex.js` remains a complete, dependency-free fallback,
and the app runs without these files. Anyone redistributing a build that bundles
them should treat that build as GPL.
