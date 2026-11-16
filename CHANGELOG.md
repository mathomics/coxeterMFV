# Changelog

Changes to published releases are recorded here. Version numbers refer to the software, independently of the article citation or archive DOI.

## Unreleased

- Prepare the official repository with a user guide, reproducibility instructions, support and contribution guidance, third-party notices, a browser container and continuous integration.
- Promote application source from working revision `bf4bcfe5ec13ea04504c7428eaf5e63822c23c8f`, including parsimonious flux variability analysis and flux interval displays.
- Rebuild the browser and Windows applications from the promoted source and record checksums in `release-manifest.json`.
- Correct Node dependency installation and native compiler requirements, align native provenance with the release version, and record known browser integration failures.
- Promote application source from working revision `79d9167656a4457809fd92098a5b46481dedbd5a`, which validates imported scenarios, bundles and bounds, keeps stale or cancelled work from replacing newer results, gives every map presentation edit an undo step, spaces out the Zuñiga map and routes the auxotroph cross-feeding lines along the outer sides of the cells.
- Move the source of both applications to the repository root, put both applications in `app/`, and add the case studies of the article (scripts, models, results and figures) in `paper/`.
- Promote application source from working revision `5fe032526bc617d79d572c61045709691f2fd4b7`, which opens on a welcome page with the three case studies, help and license, imports and exports flux data, shows scaled reduced costs, opens a case study from a link (`app.html?scenario=pub:<id>`) and fits the map to the window when a link opens it. Rebuild the browser application from it. The Windows executable is still the build from revision `79d9167656a4457809fd92098a5b46481dedbd5a` and will be rebuilt for the tagged release.
- Retry the container check on any connection error, and give Chromium 30 s to start in the browser application check.
- Publish the website at https://mathomics.github.io/coxeterMFV/, with a home page and a help page that explains how to read a solved map, built by `.github/workflows/pages.yml`.
- Promote application source from working revision `522a150d1614873eb423b27bed1470fb2c51c36b`, which turns Add Compartment into Edit Compartments: a window beside the tools sets which compartment is inside which, starting from the Automatic, Yeast9, BiGG or Empty nesting, and saves that nesting with each cell of the map (map format version 4; older maps still open). Drawn compartments are fitted into their new parent, and a parent not yet drawn is drawn around them. Rebuild the browser application from it. The Windows executable is still the build from revision `79d9167656a4457809fd92098a5b46481dedbd5a`.

## 0.1.0

The version currently declared by the JavaScript package, Rust workspace and citation file. This entry does not establish a publication date or a tagged public release. Before the first publication release, consolidate the unreleased changes, choose the final version and date, and archive that exact release.
