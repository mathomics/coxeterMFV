# Changelog

Changes to published releases are recorded here. Version numbers refer to the software, independently of the article citation or archive DOI.

## Unreleased

- Prepare the official repository with a user guide, reproducibility instructions, support and contribution guidance, third-party notices, a browser container and continuous integration.
- Promote application source from working revision `bf4bcfe5ec13ea04504c7428eaf5e63822c23c8f`, including parsimonious flux variability analysis and flux interval displays.
- Rebuild the browser and Windows applications from the promoted source and record checksums in `release-manifest.json`.
- Correct Node dependency installation and native compiler requirements, align native provenance with the release version, and record known browser integration failures.
- Promote application source from working revision `79d9167656a4457809fd92098a5b46481dedbd5a`, which validates imported scenarios, bundles and bounds, keeps stale or cancelled work from replacing newer results, gives every map presentation edit an undo step, spaces out the Zuñiga map and routes the auxotroph cross-feeding lines along the outer sides of the cells.
- Move the source of both applications to the repository root, put both applications in `app/`, and add the case studies of the article (scripts, models, results and figures) in `paper/`.

## 0.1.0

The version currently declared by the JavaScript package, Rust workspace and citation file. This entry does not establish a publication date or a tagged public release. Before the first publication release, consolidate the unreleased changes, choose the final version and date, and archive that exact release.
