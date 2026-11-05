# Yeast9 compartments

This example combines the yeast-GEM 9.1.1 reconstruction with a Coxeter-MFV map of selected cytoplasmic, mitochondrial and peroxisomal pathways. The map contains 135 drawn reactions and 408 metabolite nodes. It is an excerpt of the model's 4105 reactions, 2748 metabolites and 1143 genes.

## Open the example

1. Open the browser or desktop application.
2. Use **File > Import > Model** and select `yeastgem_v9.1.1-edited.json`.
3. Use **File > Import > Map JSON** and select `yeast9_cytosol_mitochondria_peroxisome.json`.
4. Inspect the nested compartments, choose the desired objective and conditions, and run the analysis. Save the scenario and solver information with any exported results.

The JSON model was converted from the original SBML release preserved at `../../data/yeast-GEM-9.1.1/yeast-GEM.xml`. The distributed JSON retains the same reaction count and reaction bounds as that SBML file; its biomass objective is `r_2111`. The filename is retained for compatibility. The map is a Coxeter-MFV drawing, separate from the model.

## Source and terms

The upstream model is licensed under CC BY 4.0. The full licence is preserved in [`../../data/yeast-GEM-9.1.1/LICENSE.md`](../../data/yeast-GEM-9.1.1/LICENSE.md). Credit the yeast-GEM authors and identify the conversion and map when reusing this example. The model is not relicensed under the project's MIT licence.

| File | SHA-256 |
| --- | --- |
| `yeastgem_v9.1.1-edited.json` | `1cda4ad32f400afc53cc0224a5df00d4ff6b3531274066c9b0788b3bd7f5ed24` |
| `yeast9_cytosol_mitochondria_peroxisome.json` | `99014b5378d7619988ff9bea773debfeb43b729f1ddaf45400d0a7d5c409bf06` |

Zhang C et al. (2024). Yeast9: a consensus genome-scale metabolic model for *S. cerevisiae* curated by the community. *Molecular Systems Biology* 20(10), 1134–1150.

- [Yeast9 article](https://doi.org/10.1038/s44320-024-00060-7)
- [yeast-GEM release 9.1.1](https://github.com/SysBioChalmers/yeast-GEM/releases/tag/v9.1.1)
