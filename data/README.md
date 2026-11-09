# Data

Original source data used by the examples, and where every dataset in this repository comes from.

## In this folder

| Folder | Contents | Source | License |
| --- | --- | --- | --- |
| [`yeast-GEM-9.1.1`](yeast-GEM-9.1.1) | The Yeast9 model, release 9.1.1, as SBML Level 3 with the FBC package. 4105 reactions, 2748 metabolites and 1143 genes | [SysBioChalmers/yeast-GEM](https://github.com/SysBioChalmers/yeast-GEM) | [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) |

## Elsewhere in the repository

| Data | Location | Source | Terms |
| --- | --- | --- | --- |
| Five teaching models (*E. coli*, *S. cerevisiae*, *Synechocystis*, *Methanosarcina* and *Lactococcus lactis* cores) and their scenarios | [`models`](../models) | Written for Coxeter-MFV from the descriptions in `tools/species` | MIT, as the code |
| Yeast9 in COBRA JSON format | [`examples/yeast9-cytosol-mitochondria-peroxisome`](../examples/yeast9-cytosol-mitochondria-peroxisome) | Converted from the release above | CC BY 4.0 |
| *D. vulgaris* and *M. maripaludis* | [`examples/stolyar-2007-syntrophy`](../examples/stolyar-2007-syntrophy) | Supplement of Stolyar et al. (2007) | Exact redistribution terms for the converted supplement files remain to be recorded. Cite the article. |
| Four auxotrophic *E. coli* strains | [`examples/steadycom-ecoli-auxotrophs`](../examples/steadycom-ecoli-auxotrophs) | iAF1260 with the knockouts of Chan et al. (2017) | Confirm the terms of the specific iAF1260 source and tutorial inputs. Cite both works. |
| Nine-species gut community | [`examples/steadycom-gut-nine`](../examples/steadycom-gut-nine) | Community model of Chan et al. (2017) | The article is CC BY; confirm and record the terms of the specific workbook and underlying models. |
| *S. elongatus* and *E. coli* ΔpheA | Downloaded at run time, not stored | Repository of Zuñiga et al. (2020) | Article and supplement are CC BY 4.0. No licence is inferred for the separately hosted MATLAB models, which are not redistributed. |

The maps in the examples were drawn with Coxeter-MFV for this repository.

## Provenance and redistribution

The root MIT licence does not grant rights to third-party reconstructions. Attribution to a paper is required where appropriate, but a citation alone is not a redistribution licence. The entries above explicitly identify terms that remain to be verified before the publication release. Do not describe those datasets as MIT or assume that an article's licence covers an independently hosted model.

The Yeast9 upstream licence is preserved in [`yeast-GEM-9.1.1/LICENSE.md`](yeast-GEM-9.1.1/LICENSE.md). Its [example README](../examples/yeast9-cytosol-mitochondria-peroxisome/README.md) records the model and map files. The literature manifests in `models/published/` describe member files, medium, bounds and selected source revisions. The corresponding files in `examples/` provide ready-to-import copies.

Before publishing, preserve each source URL, release or commit identifier, applicable licence or written permission, download checksum and conversion procedure in the dataset record. The publication checklist in [`docs/PUBLICATION.md`](../docs/PUBLICATION.md) includes this rights review.
