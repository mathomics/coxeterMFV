# User guide

Coxeter-MFV displays metabolic models inside explicit cell and compartment boundaries. The model determines the numerical problem; the map determines which parts are drawn and where. A curated map can show a subset of a much larger model.

## Start the application

Download `app/web/coxeter_mfv.html` and open it in a current browser, or run `app/windows/coxeter-mfv.exe` on 64-bit Windows. The browser file contains the application, GLPK and bundled teaching models. No account is required.

The normal editing and solving workflow runs locally. Downloads of external models, including the Zuñiga source models and the optional Yeast9 model, require an internet connection. Following external database or reference links also connects to those services. Save outputs explicitly before closing the application.

To serve the packaged browser app in a container, install Docker or Podman, then run these commands from the repository root:

```bash
docker build -t coxeter-mfv .
docker run --rm -p 127.0.0.1:8080:8080 coxeter-mfv
```

Open `http://127.0.0.1:8080`. The container serves the same packaged HTML; calculations still run in the browser. The container runtime and image download are prerequisites.

## First analysis with a bundled model

1. Open **Edit Model**, then **Scenario & objective**. Select a bundled teaching scenario. For a small first example, choose an *E. coli* core scenario.
2. Return to the map. Inspect the cells, compartments and reaction directions. Use the mouse wheel to navigate and the application controls to fit the map to the viewport.
3. In the simulation controls, run FBA. An optimal result should show a positive growth objective for an unmodified bundled teaching scenario. Exact objectives and balance checks can also be reproduced with `npm run validate` from `code/`.
4. Inspect reaction fluxes and the reported objective. Change a nutrient uptake bound, solve again, and observe the effect on growth and the drawn fluxes.
5. Save the scenario and map, and export the flux table. Export a reproducibility bundle when you need to preserve the numerical setup and result together.

## Open your own model and map

Use **File > Import > Model** to load a COBRA JSON, SBML, supported COBRA Toolbox MAT or Excel model. Then use **File > Import > Map JSON** to load a compatible Coxeter map. Import the model first so map identifiers can be matched against it.

A ready-made eukaryotic example is in [`examples/yeast9-cytosol-mitochondria-peroxisome`](../examples/yeast9-cytosol-mitochondria-peroxisome/README.md). Import `yeastgem_v9.1.1-edited.json`, then `yeast9_cytosol_mitochondria_peroxisome.json`. The map draws selected pathways; solving uses the imported genome-scale model.

Map operations such as moving a cell, bending a reaction curve, changing labels or hiding a node affect the drawing. Model operations such as changing bounds, stoichiometry, objectives or gene knockouts affect the mathematical problem. Save the relevant files after making either type of change.

## Choose an analysis

| Analysis | Interpretation |
| --- | --- |
| FBA | Optimizes the selected objective subject to steady-state balances and bounds. |
| pFBA | Minimizes total absolute flux after fixing a requested fraction of the primary optimum. |
| FVA | Finds feasible minimum and maximum fluxes at the selected objective requirement. |
| Parsimonious FVA | Adds a total-flux restriction derived from pFBA before finding flux intervals. |
| Loopless FBA and CycleFreeFlux | Apply additional restrictions or postprocessing to address internal cycles. Record the selected method. |
| SteadyCom | Searches for community growth compatible with a common steady-state growth rate and the specified community constraints. |
| GIMME | Uses expression evidence and an objective requirement to construct a context-specific model. |

The browser uses GLPK when initialized. The desktop application includes HiGHS and can use an installed, licensed Gurobi or CPLEX library. Solver choice and tolerances can affect the selected flux vector. Objective agreement alone does not demonstrate that every reaction flux is unique.

## Published communities

Select a literature community in **Edit Model > Scenario & objective**. The Stolyar and SteadyCom cases include the community inputs. The Zuñiga case downloads pinned MATLAB sources on demand and checks their SHA-256 values; it cannot be reproduced from the initial offline download alone.

Use the original source references when interpreting a result. A teaching core model, a preview map and a complete literature reconstruction serve different purposes. See [the example catalogue](../examples/README.md) and [the reproducibility guide](REPRODUCIBILITY.md).

## Save and share

| Output | Use |
| --- | --- |
| Map JSON | Preserve layout, cell boundaries and drawing choices. |
| Model export | Preserve the biological network in a supported exchange format. |
| Scenario JSON | Preserve analysis conditions such as bounds and objectives. |
| Reproducibility bundle | Preserve the model, configuration, result and recorded provenance for replay. |
| Flux CSV | Analyse reaction fluxes elsewhere. |
| SVG or PNG | Prepare figures. SVG retains vector geometry. |
| Methods and provenance report | Document the solver, method and recorded inputs. |

Keep the release version, solver and source data citation alongside shared outputs. Check the exported report for any limits or missing provenance. Troubleshooting and issue-reporting instructions are in [SUPPORT.md](../SUPPORT.md).
