# *Desulfovibrio vulgaris* and *Methanococcus maripaludis*

The first stoichiometric model of a microbial mutualism. *D. vulgaris* oxidises lactate to acetate, carbon dioxide and hydrogen, which is only favourable while the hydrogen stays low. *M. maripaludis* keeps it low by turning it into methane. With sulfate withheld, neither grows on lactate alone.

The joint model is read from the four CellNetAnalyzer files of the paper's supplement, with 180 reactions and 163 metabolites, its bounds, objective coefficients and the fifteen measured rates as written.

## Files

| File | Contents |
| --- | --- |
| `manifest.json` | The community, its members and its medium |
| `joint-model.json` | The joint model of the supplement |
| `model-Dv.json`, `model-Mm.json` | The two members as separate COBRA models |
| `preview-Dv.json`, `preview-Mm.json` | Maps of the two members |
| `media.csv`, `bounds.csv` | Medium and bounds of the community |

## Reference

Stolyar S, Van Dien S, Hillesland KL, Pinel N, Lie TJ, Leigh JA, Stahl DA (2007). Metabolic modeling of a mutualistic microbial community. *Molecular Systems Biology* 3, 92. https://doi.org/10.1038/msb4100131

The models are derived from the supplementary material of the article. Please cite the article when you use them.
