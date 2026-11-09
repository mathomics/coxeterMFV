# Reproduce the Zuñiga pheA case

Reference: [Zuñiga et al. (2020), Nature Communications 11, 3803](https://doi.org/10.1038/s41467-020-17612-8).
Input repository: [CZLaboratory/SyntheticMicrobialCommunities](https://github.com/CZLaboratory/SyntheticMicrobialCommunities), revision **5c53e44d56118e9ce2f197aa6c2a5ee25d63ee75**.

These scripts use the complete deposited source matrices. They do not bundle the
authors' files. Obtain those files from the authors; their public repository did
not provide a redistribution license at the audited revision. Keep local model
JSON exports outside the distributed application until permissions are verified.

## Inputs and the precise comparison

- WT monoculture: M-models.mat, SingleM{4}, unmodified E. coli iJO1366.
- Mutant monoculture: the same complete model and saved medium, with pheA
  (b2599) deleted by its original gene–reaction rule.
- Mutant coculture: CM-modelsKO.mat, modelCo2{2}, iJO1366 plus S. elongatus
  iJB785, with the same explicit b2599 deletion.

The MATLAB cell indices above are one-based. The phenotype deletion sets only
PPNDH or PPNDH_D to [0, 0]. CHORM/CHORM_D stays active because its source
GPR is b2599 or b2600.

Despite its filename, CM-modelsKO.mat does not already contain a pheA knockout.
It differs from CM-models.mat by removal of five reactions in each deposited
community: 4abutnt, EX_4abutn_[smp], EX_fald_[smp], faldAt, and faldDt.
The retained reaction bounds are identical. This is an original archive
difference, not a modification made by these scripts.

No stoichiometry, nutritional bounds, biomass definition, or objective
coefficient is changed. The community maximizes the source sum of
BIOMASS_Ec_iJO1366_core_53p95M_D and BOF_A; no common growth rate or abundance
constraint is added. Monoculture uses BIOMASS_Ec_iJO1366_core_53p95M.

The saved monoculture and coculture media differ. For example, their glucose
uptake bounds are -1 and -0.1, respectively; both allow a small citrate uptake
(-0.02). Report the three rates with these differences. They are not a
controlled experiment with an identical medium, and the deposited community
must not be described as having no organic carbon uptake.

## Python / COBRApy

Validated with Python 3.10.9 and the versions in requirements.txt.
GLPK is explicitly selected and its feasibility tolerance set to 1e-9.
Each optimum is independently checked with SciPy HiGHS using the original
sparse matrix, objective vector, and the same knockout bounds.

From the Coxeter repository root:

~~~powershell
python tools/zuniga/reproduce.py --source-dir path/to/SyntheticMicrobialCommunities --output-dir local-results/zuniga
~~~

To write complete compatible model JSON for private local use:

~~~powershell
python tools/zuniga/reproduce.py --source-dir path/to/SyntheticMicrobialCommunities --output-dir local-results/zuniga --export-local-models
~~~

To audit the alternative deposited community archive:

~~~powershell
python tools/zuniga/reproduce.py --source-dir path/to/SyntheticMicrobialCommunities --output-dir local-results/zuniga-base-archive --community-file CM-models.mat
~~~

Output contains full reaction flux CSV files, scenario summaries, input file
SHA-256 hashes, exact bounds/stoichiometry/gene-association checks, solver
residuals, and the original archive comparison. validation.json checks the
mutant growth and phenylalanine exchange against Supplementary Data 9.
It also computes FVA at 100% of the original community objective for the two
biomass reactions, the two phenylalanine transfers, PPNDH_D, and the closed
external phenylalanine exchange. This distinguishes the required rescue
mechanism from unrelated intracellular fluxes with alternative optima.

The deposited human-readable GPR for PFL/PFL_D has an unmatched parenthesis.
The Python converter uses the valid original indexed rules expression for
that reaction, translating its original gene indices. It preserves both
source expressions in the local JSON notes and records this syntax adaptation
in the report. It verifies the complete original rxnGeneMat after import;
there are no added or removed gene associations.

## MATLAB / COBRA Toolbox

Validated with MATLAB R2021a (9.10.0.1602886) and local COBRA Toolbox revision
**eb4252a4b0237eb1c7911dfbafdd32e0442dc11e**, using its working GLPK MEX interface.
The script uses deleteModelGenes with the original indexed rules, checks
that exactly one reaction is constrained, and calls optimizeCbModel without
a secondary objective. It never calls savepath or changes global installation
configuration.
The installed COBRA checkout contains local changes, including its initializer.
The tested solver, optimizeCbModel, and deleteModelGenes source files matched
the recorded revision; this script did not call the altered initializer.

~~~matlab
addpath('tools/zuniga');
reproduce_zuniga('path/to/SyntheticMicrobialCommunities', ...
                'local-results/zuniga-matlab', ...
                'path/to/cobratoolbox');
~~~

For a configured MATLAB/COBRA installation:

~~~powershell
matlab -batch "addpath('tools/zuniga'); reproduce_zuniga('path/to/SyntheticMicrobialCommunities','local-results/zuniga-matlab');"
~~~

GLPK is required for this script. Gurobi was installed on the audited computer
but its license was expired, so it was not used. CPLEX Community solved a small
test LP, but no CPLEX MATLAB/Python interface was configured; it was not used
for genome-scale calculations.

## Verified stationary predictions

The exact Coxeter JavaScript import and bundled WebAssembly solver can also be
checked locally, without installing another solver. The command reads the
original MAT files through the browser importer, validates the source hashes,
then computes all three full stationary LPs and writes only derived numerical
results. It does not export a model.

~~~powershell
node tools/zuniga/validate.mjs --source-dir path/to/SyntheticMicrobialCommunities --output-dir local-results/zuniga-js --python-results-dir local-results/zuniga --matlab-results-dir local-results/zuniga-matlab
~~~

The two independent-result directory options are optional. When supplied, the
validator compares source-array hashes, model dimensions, gene counts, growth,
and objective values against the Python and MATLAB calculations. It also checks
the complete namespace mapping, every stoichiometric coefficient, bounds,
objective coefficients, steady-state residuals, and cross-feeding direction.
Browser-solver growth and phenylalanine-transfer values matched the published
reference within 1e-8. The coculture mass-balance residual was 1.92e-11;
the largest residual across all three scenarios was 3.05e-8 in WT monoculture.
Full-vector feasibility and published-reference agreement use separate checks.

Synthetic regression fixtures, authored independently of the original model
data, can be run with:

~~~powershell
node --test test/zuniga-core.test.mjs
~~~

| Scenario | E. coli biomass flux | S. elongatus biomass flux | Objective |
| --- | ---: | ---: | ---: |
| WT reference monoculture | 0.0879717380111 | — | 0.0879717380111 |
| pheA mutant monoculture | 0 | — | 0 |
| pheA mutant coculture | 0.0775740479549 | 0.0348484100759 | 0.112422458031 |

The mutant coculture solution transfers phenylalanine through PHELtex_A
(-0.0143717559944, secretion toward the shared compartment) and PHEtex_D
(+0.0143717559944, uptake toward E. coli). The external phenylalanine exchange
EX_phe__L_[smp] remains closed; no exogenous uptake was introduced.

The mutant growth and transfer values match Supplementary Data 9 within
1e-8 absolute tolerance. Python GLPK, SciPy HiGHS, and MATLAB/COBRA GLPK
objectives agree within 5e-13. The largest observed MATLAB steady-state
residual was 1.49e-12; Python GLPK's was 1.71e-13.

The WT reference-medium calculation is an explicit comparison added here,
not a published experimental growth-rate reproduction. This stationary
gene-deletion analysis does not reproduce dynamic culture trajectories,
spatial interactions, or measured experimental growth curves. Other
intracellular fluxes can have alternative optima; do not interpret one
reported full flux vector as evidence of a unique pathway activity.
