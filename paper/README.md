# Case studies of the article

The scripts, models, solutions and figures behind the figures and Supplementary Table S2 of the Coxeter-MFV article. Every script loads the application's own modules from [`../src`](../src), so each result comes from the same code the applications run, not from a separate reimplementation.

## Figures

| Figure | What it shows | Scripts | Outputs |
| --- | --- | --- | --- |
| 1 | How a model and its map share identifiers, nested compartments in two cells, and the web application | [`figures/compose_figures.py`](figures/compose_figures.py), from [`fig1_schematic.svg`](figures/fig1_schematic.svg) and the screenshot [`fig1_interface.png`](figures/fig1_interface.png) | `figures/out/Figure1.*` |
| 2 | FBA, pFBA, CycleFreeFlux and FVA of Yeast9 on its cytoplasm and mitochondrion map | [`yeast9/solver_comparison.mjs`](yeast9/solver_comparison.mjs), which draws the added reactions with [`yeast9/figure_amino_acids.mjs`](yeast9/figure_amino_acids.mjs), then [`yeast9/verify_solver_comparison.py`](yeast9/verify_solver_comparison.py) and [`yeast9/compose_solver_figure.py`](yeast9/compose_solver_figure.py) | `yeast9/results/solver-comparison/`, `figures/out/Figure2.*` |
| 3 | Phenylalanine rescue of *E. coli* ΔpheA by *S. elongatus* (Zuñiga et al. 2020) | [`zuniga/coxeter_zuniga.mjs`](zuniga/coxeter_zuniga.mjs), then `figures/compose_figures.py` | `zuniga/results/`, `figures/out/Figure3.*` |
| 4 | SteadyCom community of four auxotrophic *E. coli* strains (Chan et al. 2017) | [`steadycom/coxeter_steadycom.mjs`](steadycom/coxeter_steadycom.mjs), then `figures/compose_figures.py` | `steadycom/results/`, `figures/out/Figure4.*` |

The method details of Figure 2, including the reactions added to the map, are in [`yeast9/results/solver-comparison/README.md`](yeast9/results/solver-comparison/README.md).

## Checks reported in Supplementary Table S2

| Check | Where |
| --- | --- |
| Yeast9 growth, minimum total flux and the 256 FVA endpoints of the drawn reactions against COBRApy | `yeast9/verify_solver_comparison.py` writes `yeast9/results/solver-comparison/cobrapy_validation.json` |
| The Yeast9 COBRA JSON holds the same reactions, coefficients, bounds and objective as the yeast-GEM 9.1.1 SBML | `yeast9/build_anaerobic_model.py` checks this before it writes the anaerobic model |
| Zuñiga growth rates and phenylalanine transfer against Supplementary Data 9 of the original article, COBRApy and the COBRA Toolbox | [`../tools/zuniga`](../tools/zuniga) (`reproduce.py`, `reproduce_zuniga.m` and `validate.mjs`) |
| SteadyCom maximum community growth and the abundance range of each strain | `steadycom/results/coxeter_steadycom_summary.json` |

The [`yeast9`](yeast9) folder also holds an aerobic and anaerobic pFBA comparison with COBRApy (`cobrapy_reference.py`, `build_anaerobic_model.py`, `coxeter_pfba.mjs`, `compare_fluxes.py` and `alternative_optima.py`, with their results). It makes the anaerobic model with yeast-GEM's own conversion at commit 2d594ae, kept in [`yeast9/yeast-gem-2d594ae`](yeast9/yeast-gem-2d594ae) under the yeast-GEM licence ([CC BY 4.0](../data/yeast-GEM-9.1.1/LICENSE.md)).

## Inputs

- Yeast9: the yeast-GEM 9.1.1 SBML in [`../data/yeast-GEM-9.1.1`](../data/yeast-GEM-9.1.1), its COBRA JSON and the map in [`yeast9/models`](yeast9/models), and the curated map in [`../models/maps`](../models/maps).
- Zuñiga: the authors' original MAT files, which the script downloads from their repository and checks against pinned hashes. They are not redistributed here.
- SteadyCom: the four strain models and the map bundled in [`../models/published/steadycom-ecoli-auxotrophs`](../models/published/steadycom-ecoli-auxotrophs).

## Running

Install the locked Node dependencies once from the repository root with `npm ci`. The Python scripts need COBRApy 0.29 with GLPK, matplotlib and Pillow, and the figure scripts need Firefox on the `PATH` to render each figure to PNG. Each script writes next to itself, so it can be run from any folder. From the repository root:

```bash
node paper/yeast9/solver_comparison.mjs
python paper/yeast9/verify_solver_comparison.py
python paper/yeast9/compose_solver_figure.py
node paper/zuniga/coxeter_zuniga.mjs
node paper/steadycom/coxeter_steadycom.mjs
python paper/figures/compose_figures.py
```

The Zuñiga script needs internet access for the source models. `fig1_interface.png` is a screenshot of the web application, taken as described at the top of `compose_figures.py`.

## References

- Chan SHJ, Simons MN, Maranas CD (2017). SteadyCom: Predicting microbial abundances while ensuring community stability. *PLoS Computational Biology* 13(5), e1005539. https://doi.org/10.1371/journal.pcbi.1005539
- Zhang C et al. (2024). Yeast9: a consensus genome-scale metabolic model for *S. cerevisiae* curated by the community. *Molecular Systems Biology* 20(10), 1134–1150. https://doi.org/10.1038/s44320-024-00060-7
- Zuñiga C, Li T, Guarnieri MT, Jenkins JP, Li CT, Bingol K, Kim YM, Betenbaugh MJ, Zengler K (2020). Synthetic microbial communities of heterotrophs and phototrophs facilitate sustainable growth. *Nature Communications* 11, 3803. https://doi.org/10.1038/s41467-020-17612-8
