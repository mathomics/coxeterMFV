# Contributing

This is the official distribution and source archive of Coxeter-MFV. Application development takes place in EscherMultiCellMultiCompartment. Changes to application behaviour should be developed and tested there, then promoted here as a reviewed snapshot with matching application builds.

Use this repository's issue tracker for reproducible bugs, documentation corrections, feature requests and questions about published releases. For private vulnerability reports, follow [SECURITY.md](SECURITY.md).

## Reporting a problem

Include the release version, operating system, browser or desktop application, selected solver, steps to reproduce, and the expected and observed results. A small model, map or reproducibility bundle is particularly useful. Share only files you have permission to publish. See [SUPPORT.md](SUPPORT.md) for troubleshooting.

## Preparing a change

Documentation fixes can be proposed directly here. For application changes, identify the working repository's source commit and explain the resulting behaviour. Keep changes focused and use straightforward code consistent with the surrounding modules. Include tests for changes to numerical methods, file formats or application behaviour.

From `code/`, install the locked dependencies with `npm ci`, then run `npm test` and `npm run validate`. Native changes also need the relevant Rust tests. Follow [the release procedure](docs/RELEASING.md) when promoting source, changing dependencies or replacing an application build.

Do not edit generated HTML, model catalogues or executable files by hand. Do not include manuscript drafts, private source models, solver licences, credentials, temporary results or build caches in a promotion.

Contributions must be compatible with the project's MIT licence. Third-party components and datasets retain their own terms, as described in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Keep discussion respectful and focused on the work.
