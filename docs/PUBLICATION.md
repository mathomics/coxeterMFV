# Publication preparation

This repository is the official application distribution. EscherMultiCellMultiCompartment remains the development workspace. Application code, distributed builds, examples and release documentation belong here; manuscript drafts and development logs remain in the working repository.

This checklist supports submission preparation. It does not claim that the application has been accepted by a journal or that all publication requirements have been met.

## Repository preparation

- [x] Include browser and Windows applications with their source and locked dependency files.
- [x] Provide installation instructions, a user guide, a minimal numerical example and reproducibility instructions.
- [x] Include software citation metadata without an invented article reference or DOI.
- [x] Distinguish the MIT project source, GPL-containing browser distribution and third-party data terms.
- [x] Add automated checks and a container recipe for the packaged browser application.
- [x] Record source promotion provenance and application checksums.

## Decisions and external actions before submission

- [ ] Resolve the broader source-browser integration failures recorded in [VALIDATION.md](VALIDATION.md), distinguishing application defects from obsolete or timing-dependent assertions.
- [ ] Confirm the final software version, release date, author list, affiliations, ORCIDs and maintainer contact. Replace the collective attribution in `CITATION.cff` with the agreed software authors when available.
- [ ] Verify redistribution permissions for every literature-derived model. A request to cite an article does not by itself grant redistribution rights. Record the exact applicable licence, permissions or upstream terms in `data/README.md`.
- [ ] Complete third-party source and licence distribution for each binary, including GLPK corresponding source and native dependency notices. Preserve the exact upstream revisions and build instructions.
- [x] Curate the scripts, parameters, input identifiers and outputs behind every manuscript figure and table, in [`paper`](../paper).
- [ ] Archive the case studies in `paper/` with the exact application release or link their permanent dataset records.
- [ ] Complete the platform and public-download validation matrix beyond the Windows and Chromium/Firefox smoke checks already recorded in [VALIDATION.md](VALIDATION.md). Record actual results and resource requirements. Test the container on a host with Docker or Podman.
- [ ] Choose the target journal and submission route. Confirm its current requirements with the editors where the application does not fit a conventional web server.
- [ ] Provide a stable, freely accessible application URL if submitting as a web resource. A download link alone should not be represented as a hosted, tested service.
- [ ] Agree responsibility and resources for maintaining access and functionality for at least five years if submitting to NAR.
- [ ] Enable the intended public support and private vulnerability-reporting channels on GitHub.
- [ ] Create a versioned release, archive the exact source and required data with a permanent DOI, and verify that reviewers can access it. NAR's current detailed software instructions specifically request Figshare archiving; confirm the applicable policy before choosing another service.
- [ ] Include the archive DOI, source URL, software version and data identifiers in the manuscript's Data Availability statement and references. Add the article DOI and preferred citation only when assigned.
- [ ] Review the journal's manuscript disclosure requirements, including the use of AI tools in software or manuscript preparation, with the corresponding author.

## Availability statement preparation

The final statement should identify this repository, the exact release tag and commit, the permanent archive DOI, the browser application URL or download, and the separately licensed or externally downloaded data. It should describe where the analysis scripts and figure outputs are archived. Do not submit placeholder DOIs or claim that all inputs are bundled when a case downloads third-party models.

The software archive DOI identifies a release; the article DOI identifies the publication. Retain both when available. Confirm authorship and publication metadata with the corresponding author before changing the citation file.

## Journal guidance checked on 4 November 2026

NAR's policies cover free access, long-term availability, computational resources, code archiving, documentation and data availability. Its Web Server Issue also has route-specific requirements for websites and standalone software, including a container for standalone applications. A repository cleanup cannot establish eligibility or replace editorial review.

- [NAR data deposition and standardization](https://academic.oup.com/nar/pages/data_deposition_and_standardization)
- [NAR general policies](https://academic.oup.com/nar/pages/Policies)
- [NAR Web Server Issue submission guidance](https://academic.oup.com/nar/pages/Submission_Webserver)
