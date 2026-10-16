# coxeter-app — design

The native application. It replaces `src/ui/` of the JavaScript app:
`store.js` (state and every action), `app.js` (painting, pointer gestures,
event wiring, exports), `panels.js` (the HTML of every panel) and the small
menu controllers.

## Three layers

```
coxeter-state    the Store: what the app knows and everything it can do   (no egui)
coxeter-canvas   the map widget: camera, painting, hit-testing, gestures  (egui)
coxeter-app
  panels/        top bar, left panel, inspector, results, menus, dialogs  (egui)
  app.rs         eframe::App: wires the three, shortcuts, theme, file dialogs
  main.rs        command line: open files, `--screenshot`, `--version`
```

The state and the canvas are crates of their own, so the Store cannot reach
egui, the canvas reaches the Store only through the `MapHost` trait it
declares (the store methods its gestures call, by their JavaScript names), and
each builds and tests on its own. Below, `state/` and `canvas/` name those two
crates.

### `state/` — the Store, without a GUI

A port of `Store` in `src/ui/store.js`: the same fields, the same methods
(snake_case), the same rules. It owns the models, the `CommunityModel`, the
`FluxProblem`, the `EditableMap` with its `MapHistory`, the solution, FVA
ranges, cross-feeding, omics, pathways, selection and view options, the log
and the status line. It never draws and never opens a dialog, so the JS store
tests port to plain `#[test]`s.

Differences forced by the platform, and nothing else:

- **Files instead of `fetch`.** An `Assets` trait gives the bundled files
  (`models/**`, embedded in the binary like the single-file HTML embeds them)
  and a directory override for development. Downloads (Yeast9 SBML, the
  Zuñiga sources) go through an injectable fetcher with a disk cache.
- **Threads instead of Workers and `await frame()`.** Long work (FVA,
  SteadyCom range and solve, loopless MILP, full-map layout, GIMME) runs as a
  background job with progress and cancel; the store exposes `poll_jobs()`
  which the app calls each frame and tests call in a loop. Results are applied
  on the caller's thread, guarded by the same generation counters the JS uses
  (`_fvaGeneration`, `_steadyComGeneration`), so a stale result is dropped.
- **Events.** `emit(reason)` becomes a list of reasons since the last frame
  (`take_events()`), which the views use to know what to rebuild.

### `canvas/` — the map

- **What is drawn:** `coxeter_render::render_map(&MapGraph, &RenderState)`
  gives the SVG element tree the JS app puts on its stage. The canvas paints
  that tree with egui, using the CSS resolver of `coxeter-render` for the
  styles of `assets/map.css`. One code path draws the screen and the exports.
- **Speed:** the tree is rebuilt only when the map or the render state
  changes (a revision counter), tessellated once into meshes in map
  coordinates, and drawn each frame with the camera transform; panning and
  zooming re-tessellate nothing. While dragging, only the moved elements are
  rebuilt (the layers already carry `data-rxn` / `data-node`). Labels are
  culled by zoom as `.is-far` does. Target: 60 fps pan/zoom on the full
  iAF1260 map (45k elements), where the JS app manages 12 fps and stalls.
- **Camera:** `camera-zoom.js` and `wheel-navigation.js`, ported.
- **Hit-testing:** the same order the DOM gives (last painted wins), on the
  elements that carry `data-*` attributes and the invisible hit targets.
- **Gestures:** one state machine with the JS gestures, each a
  begin / update / finish triple: pan, node drag, reaction drag, rotate,
  control point, metabolite label, text, selection rectangle, group drag,
  cell draw, compartment draw, canvas resize, geometry (membrane) drag,
  compartment label drag, reaction pick, pinch. Each edit goes through
  `store.begin_map_edit` / `commit_map_edit`, so undo works as in JS.
- **Overlays:** selection rectangle, draw previews, the in-place text editor,
  the add-reaction panel and the context menus (metabolite, compartment,
  boundary, add-metabolite).

### `panels/` — everything around the map

The layout of the JS app (see `index.html` and `assets/app.css`): top bar with
File and Simulate menus, left panel with Edit Map / Edit Model / Simulate tabs
and the tool grid, canvas toolbar, results panel (Analysis results, Table
view, Logs), inspector (Inspector, Layout), and the workspace pages. Light and
dark themes with the colours of `app.css`. Every `data-action` of the JS app
is listed in `CHECKLIST.md`; each becomes a call into the store.

## Tests

- `state/`: the JS store tests, ported, plus parity fixtures written by the
  JS `Store` in Node (`rust/tools/fixtures-app.mjs`).
- `canvas/`: gesture tests that drive the state machine with synthetic
  pointer events and compare the map with what the JS functions produce.
- Whole app: `egui_kittest` drives the real interface offscreen (click by
  label, type, drag) and takes snapshots through wgpu; no window is opened
  on the developer's display. `coxeter --screenshot out.png [files…]` renders
  one frame for visual comparison with the JS app.

## `coxeter-app` as built

```
src/
  main.rs        command line (files…, --screenshot, --size, --scenario, --dark/--light, --models, --version)
  lib.rs         the modules below, as a library the tests drive
  app.rs         Shell (Store + UiState + MapView + dialogs + menus) and its frame:
                 poll_jobs, take_events → map revision, debounce / scheduleSolve,
                 the window keydown, the status-line notice clock; `impl eframe::App for Shell`
  actions.rs     openWorkspace, activateTool, setTheme, camera, dock toggles,
                 queueScenarioSelection, every import and export
  host.rs        `impl MapHost for StoreHost` (the Store as the map widget sees it)
  ui_state.rs    the JS `ui` object and the classes of `.app`
  widgets.rs     app.css controls with the browser's `input` / `change` events
  theme.rs       app.css colours (light / dark) as an egui style
  fonts.rs       system fonts (text, bold, mono, symbol fallbacks) via fontdb
  files.rs       FileDialogs: native (rfd) or scripted (tests, --screenshot); file kinds
  assets.rs      models/** from a folder, the repository, or compiled in (build.rs)
  panels/        topbar, left, toolbar, results, right, workspace (the page and its cards),
                 inspector, scenario, model, map_options, cross_feeding, menus, add_reaction
```

Immediate mode replaces the JS event delegation: each control does what the
`onSidebarClick` / `onSidebarInput` case for its `data-action` does, where it
is drawn. Everything is drawn every frame; only the map's drawing is cached,
and the Shell bumps its revision when the Store emits a reason other than
`status` / `variability`. Background jobs report through `poll_jobs`, so the
frame asks for a repaint while any runs.

A release build (or `--features embed-models`) compiles `models/**` into the
binary (`EmbeddedAssets`); a development build reads the repository's.
Downloads (Yeast9, the Zuñiga sources) go through the HTTPS fetcher of
`coxeter-analysis`, cached under the user's cache folder.

Deviations forced by the platform: a cancelled save dialog writes nothing and
marks nothing saved (a browser download always "succeeds"); the scenario list
loads on change (no pending-load queue to await on Return to map); ▶ ↖ ↘ ↕
and the compass arrows are drawn with stand-ins (`fonts::glyph`), as the
system fonts outline them only in a colour emoji face; the solver line says
HiGHS; files dropped on the window open as on the command line.
