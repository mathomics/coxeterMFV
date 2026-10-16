# Examples

Maps and models that show what Coxeter-MFV draws and computes, from a single eukaryotic cell to communities of nine species. The four literature communities reproduce published results and are also bundled with both applications, where they open from the scenario list.

| Folder | Organisms | Compartments | Reference |
| --- | --- | --- | --- |
| [`yeast9-cytosol-mitochondria-peroxisome`](yeast9-cytosol-mitochondria-peroxisome) | *Saccharomyces cerevisiae* | Six nested compartments in one cell | Zhang et al. (2024) |
| [`stolyar-2007-syntrophy`](stolyar-2007-syntrophy) | *Desulfovibrio vulgaris* and *Methanococcus maripaludis* | Two cells in a shared medium | Stolyar et al. (2007) |
| [`steadycom-ecoli-auxotrophs`](steadycom-ecoli-auxotrophs) | Four auxotrophic *Escherichia coli* strains | Four cells in a shared medium | Chan et al. (2017) |
| [`steadycom-gut-nine`](steadycom-gut-nine) | Nine gut bacteria | Nine cells in a shared medium | Chan et al. (2017) |
| [`zuniga-2020-phea-rescue`](zuniga-2020-phea-rescue) | *Synechococcus elongatus* and *E. coli* ΔpheA | Two cells in a shared medium | Zuñiga et al. (2020) |

## Opening an example

In either application, import the model first with **File › Import › Model**, then the map with **File › Import › Map JSON**. The literature communities need no import, since they are already in the scenario list of the **Simulate** tab.

Please cite the original work alongside Coxeter-MFV when you use an example. The literature communities list their reference and the terms of their data in their own folder.

## Yeast9 reference

The yeast example holds the Yeast9 model, release 9.1.1, in COBRA JSON format, and a map of 115 reactions drawn over it with Coxeter-MFV. The model is yeast-GEM, released by its authors under the [Creative Commons Attribution 4.0 International](https://creativecommons.org/licenses/by/4.0/) license. Its original SBML release is in [`../data/yeast-GEM-9.1.1`](../data/yeast-GEM-9.1.1).

Zhang C et al. (2024). Yeast9: a consensus genome-scale metabolic model for *S. cerevisiae* curated by the community. *Molecular Systems Biology* 20(10), 1134–1150. https://doi.org/10.1038/s44320-024-00060-7
