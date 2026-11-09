"""Recalculate the published source matrices without changing their media or objective.

This program does not distribute the authors' model files. Supply a local checkout
of CZLaboratory/SyntheticMicrobialCommunities. MATLAB cell indices are one-based:
SingleM{4} is E. coli iJO1366 and modelCo2{2} in CM-modelsKO.mat is the
iJO1366 + iJB785 pair used by the authors' knockout analysis. Despite its
filename, that archive still has an active PPNDH_D; apply b2599 explicitly.
The only biological adaptation is a GPR-based deletion of pheA (b2599).
These stationary, summed-biomass FBA scenarios are an explicit adaptation;
they are not claimed to reproduce experimental rates or dynamic trajectories.
"""
from __future__ import annotations

import argparse
import ast
import hashlib
import json
import platform
import re
import sys
from pathlib import Path

import cobra
import numpy as np
import scipy
from cobra.util.array import create_stoichiometric_matrix
from scipy import io, optimize, sparse


def text(value):
    if isinstance(value, str):
        return value
    a = np.asarray(value)
    if a.size == 0:
        return ""
    if a.size == 1:
        return text(a.item())
    return " ".join(text(x) for x in a.ravel())


def cell_strings(values):
    return [text(x) for x in np.asarray(values, dtype=object).ravel()]


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def array_hash(array):
    a = np.ascontiguousarray(np.asarray(array, dtype="<f8"))
    return hashlib.sha256(a.tobytes()).hexdigest()


def source_signature(raw):
    S = sparse.coo_matrix(raw["S"])
    order = np.lexsort((S.col, S.row))
    triples = np.column_stack((S.row[order], S.col[order], S.data[order]))
    return {
        "reactionCount": S.shape[1],
        "metaboliteCount": S.shape[0],
        "matrixNonzeros": S.nnz,
        "stoichiometrySha256": array_hash(triples),
        "lowerBoundsSha256": array_hash(raw["lb"]),
        "upperBoundsSha256": array_hash(raw["ub"]),
        "objectiveSha256": array_hash(raw["c"]),
        "rhsSha256": array_hash(raw["b"]),
    }


def compartment(metabolite_id):
    # Preserve original compartment and species suffix, and the shared [smp].
    shared = re.search(r"\[([^\]]+)\]$", metabolite_id)
    if shared:
        return shared.group(1)
    match = re.search(r"_([A-Za-z0-9]+)(_[ABCD])?$", metabolite_id)
    if match:
        return match.group(1) + (match.group(2) or "")
    raise ValueError(f"Cannot infer the source compartment of {metabolite_id!r}")


def convert(raw, model_id):
    # COBRApy's standard .mat loader ignores the b/csense fields. Verify the
    # exact source constraint form before using its steady-state model class.
    S = sparse.csc_matrix(raw["S"])
    b = np.asarray(raw["b"], dtype=float).ravel()
    csense = text(raw.get("csense", "E" * S.shape[0])).replace(" ", "")
    if np.any(b != 0) or csense != "E" * S.shape[0]:
        raise ValueError("Source has nonzero RHS or non-equality rows; refusing to alter them")
    rxns, mets = cell_strings(raw["rxns"]), cell_strings(raw["mets"])
    names = cell_strings(raw.get("metNames", raw["mets"]))
    formulas = cell_strings(raw.get("metFormulas", [""] * len(mets)))
    model = cobra.Model(model_id)
    new_mets = [
        cobra.Metabolite(mid, name=names[i], formula=formulas[i] or None,
                         compartment=compartment(mid))
        for i, mid in enumerate(mets)
    ]
    model.add_metabolites(new_mets)
    rxn_names = cell_strings(raw.get("rxnNames", raw["rxns"]))
    rules = cell_strings(raw.get("grRules", [""] * len(rxns)))
    indexed_rules = cell_strings(raw.get("rules", [""] * len(rxns)))
    source_genes = cell_strings(raw["genes"])
    # Preserve source genes that happen to have no associated reaction.
    for gene_id in source_genes:
        gene = cobra.Gene(gene_id)
        gene._model = model
        model.genes += [gene]
    subsystems = cell_strings(raw.get("subSystems", [""] * len(rxns)))
    lb, ub, c = [np.asarray(raw[k], dtype=float).ravel() for k in ("lb", "ub", "c")]
    reactions = []
    for j, rid in enumerate(rxns):
        r = cobra.Reaction(rid, name=rxn_names[j], lower_bound=float(lb[j]),
                           upper_bound=float(ub[j]))
        column = S.getcol(j)
        r.add_metabolites({new_mets[i]: float(x) for i, x in zip(column.indices, column.data)})
        rule = rules[j]
        # PFL's deposited grRules has one unmatched parenthesis. Its original
        # indexed rules is valid and authoritative for COBRA Toolbox deletions.
        # Translate that saved expression; do not invent a repaired GPR.
        if rule:
            try:
                ast.parse(rule, mode="eval")
            except SyntaxError:
                indexed = indexed_rules[j]
                if not indexed:
                    raise ValueError(f"Invalid source GPR without indexed equivalent: {rid}")
                rule = re.sub(r"x\((\d+)\)", lambda match: source_genes[int(match.group(1)) - 1], indexed)
                rule = rule.replace("&", " and ").replace("|", " or ")
                ast.parse(rule, mode="eval")
                r.notes["source_grRule"] = rules[j]
                r.notes["source_indexed_rule"] = indexed
                r.notes["import_adaptation"] = "Valid original indexed rules substituted for malformed grRules"
        r.gene_reaction_rule = rule
        r.subsystem = subsystems[j]
        reactions.append(r)
    model.add_reactions(reactions)
    if set(source_genes) != {g.id for g in model.genes}:
        raise AssertionError("Converted gene IDs differ from original")
    gene_index = {gene_id: i for i, gene_id in enumerate(source_genes)}
    gene_matrix = sparse.coo_matrix(raw["rxnGeneMat"])
    source_associations = set(zip(gene_matrix.row, gene_matrix.col))
    converted_associations = {
        (i, gene_index[g.id]) for i, r in enumerate(model.reactions) for g in r.genes
    }
    if source_associations != converted_associations:
        raise AssertionError("Converted reaction-gene associations differ from original rxnGeneMat")
    model.objective = {model.reactions[j]: float(coefficient)
                       for j, coefficient in enumerate(c) if coefficient != 0}
    # Neither source model defines a minimization sense. The authors use the
    # default maximize biomass call; retain it here.
    model.objective_direction = "max"
    model.solver = "glpk"
    model.solver.configuration.tolerances.feasibility = 1e-9
    converted_S = sparse.csc_matrix(create_stoichiometric_matrix(model, array_type="lil"))
    if (converted_S - S).nnz != 0:
        raise AssertionError("Converted stoichiometry differs from original")
    if rxns != [r.id for r in model.reactions] or mets != [m.id for m in model.metabolites]:
        raise AssertionError("Source order was changed")
    if not np.array_equal(lb, [r.lower_bound for r in model.reactions]):
        raise AssertionError("Converted lower bounds differ from original")
    if not np.array_equal(ub, [r.upper_bound for r in model.reactions]):
        raise AssertionError("Converted upper bounds differ from original")
    return model


def changed_bounds(before, after):
    return [
        {"reaction": rid, "before": list(bounds), "after": list(after.reactions.get_by_id(rid).bounds)}
        for rid, bounds in before.items()
        if bounds != after.reactions.get_by_id(rid).bounds
    ]


def run_scenario(raw, scenario_id, knockout, output_dir, export_local_models=False):
    model = convert(raw, scenario_id)
    before = {r.id: r.bounds for r in model.reactions}
    if knockout:
        if "b2599" not in model.genes:
            raise AssertionError("Published pheA gene b2599 absent from model")
        model.genes.get_by_id("b2599").knock_out()
    modifications = changed_bounds(before, model)
    solution = model.optimize()
    if solution.status != "optimal":
        raise RuntimeError(f"{scenario_id}: {solution.status}")
    flux = np.asarray(solution.fluxes, dtype=float)
    S = sparse.csc_matrix(raw["S"])
    lb = np.asarray([r.lower_bound for r in model.reactions])
    ub = np.asarray([r.upper_bound for r in model.reactions])
    c = np.asarray(raw["c"], dtype=float).ravel()
    independent = optimize.linprog(
        -c, A_eq=S, b_eq=np.asarray(raw["b"], dtype=float).ravel(),
        bounds=list(zip(lb, ub)), method="highs",
        options={"primal_feasibility_tolerance": 1e-9, "dual_feasibility_tolerance": 1e-9},
    )
    if not independent.success:
        raise RuntimeError(f"SciPy HiGHS independent check failed: {independent.message}")
    if abs(solution.objective_value + independent.fun) > 1e-7:
        raise AssertionError("COBRApy/GLPK and SciPy/HiGHS objectives disagree")
    selected = {}
    for r in model.reactions:
        if (r.objective_coefficient != 0 or "phe" in r.id.lower()
                or r.id.startswith(("PPNDH", "CHORM", "EX_glc", "EX_co2",
                                     "EX_hco3", "EX_o2", "EX_no3", "EX_nh4"))
                or "biomass" in r.id.lower() or r.id == "BOF_A"):
            selected[r.id] = {
                "flux": float(solution.fluxes[r.id]),
                "bounds": list(r.bounds),
                "objectiveCoefficient": r.objective_coefficient,
                "gpr": r.gene_reaction_rule,
                "equation": r.reaction,
            }
    exchanges = {}
    S_csc = sparse.csc_matrix(raw["S"])
    for j, r in enumerate(model.reactions):
        if r.id.startswith("EX_") or S_csc.getcol(j).nnz == 1:
            exchanges[r.id] = {
                "flux": float(flux[j]), "bounds": list(r.bounds),
                "equation": r.reaction,
            }
    growth = {r.id: float(solution.fluxes[r.id])
              for r in model.reactions if r.objective_coefficient != 0}
    result = {
        "id": scenario_id,
        "calculation": "stationary FBA; maximize original source c vector",
        "status": solution.status,
        "objective": float(solution.objective_value),
        "growthObjectiveReactions": growth,
        "knockoutGene": "b2599" if knockout else None,
        "modifiedBounds": modifications,
        "source": source_signature(raw),
        "geneCount": len(model.genes),
        "gprImportAdaptations": [
            {"reaction": r.id, "sourceGrRule": r.notes["source_grRule"],
             "sourceIndexedRule": r.notes["source_indexed_rule"],
             "parsedRule": r.gene_reaction_rule,
             "reason": r.notes["import_adaptation"]}
            for r in model.reactions if "import_adaptation" in r.notes
        ],
        "medium": exchanges,
        "selectedReactions": selected,
        "validation": {
            "stoichiometryAndOriginalBoundsExact": True,
            "allSourceGenesAndReactionGeneAssociationsExact": True,
            "maxMassBalanceResidual": float(np.max(np.abs(S @ flux))),
            "maxBoundViolation": float(max(0, np.max(lb - flux), np.max(flux - ub))),
            "knockoutMaxFlux": max((abs(solution.fluxes[x["reaction"]]) for x in modifications), default=0),
            "independentSolver": "SciPy HiGHS",
            "solverFeasibilityTolerance": 1e-9,
            "independentObjective": float(-independent.fun),
            "independentMaxMassBalanceResidual": float(np.max(np.abs(S @ independent.x))),
            "independentMaxBoundViolation": float(max(0, np.max(lb - independent.x), np.max(independent.x - ub))),
        },
        "interpretation": (
            "Source model with an explicit GPR pheA knockout adaptation; medium and objective "
            "are unchanged. This is not a claim of reproducing a published growth value."
            if knockout else
            "Unmodified published source model under its saved bounds and objective; "
            "not a claim of reproducing a published experimental growth value."
        ),
    }
    if knockout and "PPNDH_D" in model.reactions:
        mechanism_reactions = [
            "BIOMASS_Ec_iJO1366_core_53p95M_D", "BOF_A", "PHEtex_D",
            "PHELtex_A", "PPNDH_D", "EX_phe__L_[smp]",
        ]
        fva = cobra.flux_analysis.flux_variability_analysis(
            model, reaction_list=mechanism_reactions,
            fraction_of_optimum=1, processes=1,
        )
        result["mechanismFvaAtOriginalObjective"] = {
            "method": "FVA at 100% of the unchanged source objective; validation analysis, not a new medium",
            "reactions": fva.to_dict(orient="index"),
        }
    output_dir.mkdir(parents=True, exist_ok=True)
    (output_dir / f"{scenario_id}.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    solution.fluxes.to_csv(output_dir / f"{scenario_id}-fluxes.csv", header=["flux"])
    if export_local_models:
        # These are local derivative files, not authorized redistribution.
        cobra.io.save_json_model(model, str(output_dir / f"{scenario_id}-model.json"), pretty=True)
    return result


def compare_knockout_archive(wt, ko):
    comparison = []
    for i, (a, b) in enumerate(zip(wt, ko), 1):
        aa, bb = set(cell_strings(a["rxns"])), set(cell_strings(b["rxns"]))
        ar = {r: j for j, r in enumerate(cell_strings(a["rxns"]))}
        br = {r: j for j, r in enumerate(cell_strings(b["rxns"]))}
        bound_changes = []
        for rid in sorted(aa & bb):
            ab = [float(a[k][ar[rid]]) for k in ("lb", "ub")]
            kb = [float(b[k][br[rid]]) for k in ("lb", "ub")]
            if ab != kb:
                bound_changes.append({"reaction": rid, "wt": ab, "archiveKO": kb})
        comparison.append({
            "matlabCellIndex": i,
            "wtDescription": text(a.get("description", "")),
            "archiveKODescription": text(b.get("description", "")),
            "removedReactions": sorted(aa - bb),
            "addedReactions": sorted(bb - aa),
            "boundChangesAmongRetained": bound_changes,
        })
    return comparison


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--community-file", choices=["CM-modelsKO.mat", "CM-models.mat"], default="CM-modelsKO.mat")
    parser.add_argument("--export-local-models", action="store_true", help="Write complete compatible model JSON locally; verify upstream redistribution rights before publishing")
    args = parser.parse_args()
    mono_path = args.source_dir / "M-models.mat"
    co_path = args.source_dir / args.community_file
    ko_path = args.source_dir / "CM-modelsKO.mat"
    mono = io.loadmat(mono_path, simplify_cells=True)["SingleM"][3]
    communities = io.loadmat(co_path, simplify_cells=True)["modelCo2"]
    co = communities[1]
    expected = ("iJO1366", "iJB785")
    if expected[0] not in text(mono["description"]) or any(x not in text(co["description"]) for x in expected):
        raise AssertionError("Expected source model descriptions do not match selected MATLAB cells")
    results = [
        run_scenario(mono, "wt-monoculture", False, args.output_dir, args.export_local_models),
        run_scenario(mono, "mutant-monoculture", True, args.output_dir, args.export_local_models),
        run_scenario(co, "mutant-coculture", True, args.output_dir, args.export_local_models),
    ]
    report = {
        "schemaVersion": 1,
        "runtime": {"python": sys.version, "platform": platform.platform(),
                    "cobra": cobra.__version__, "scipy": scipy.__version__,
                    "solver": "GLPK through COBRApy; independent SciPy HiGHS",
                    "feasibilityTolerance": 1e-9},
        "inputs": {
            mono_path.name: {"sha256": sha256(mono_path), "selectedCell": "SingleM{4}"},
            co_path.name: {"sha256": sha256(co_path), "selectedCell": "modelCo2{2}"},
        },
        "scenarios": results,
        "comparisonLimitations": [
            "Original saved monoculture and community media differ; compare growth with those explicit differences.",
            "The community maximizes the sum of two biomass reactions; no equal growth or abundance constraint is added.",
            "A selected optimum does not establish a unique cross-feeding flux; no new exporter or uptake bound is introduced.",
            "The three stationary scenarios are an explicit adaptation of the deposited models, not a reproduction of a dynamic experiment.",
        ],
        "publishedReference": {
            "source": "Zuñiga et al. (2020), Supplementary Data 9, pheA deletion; https://doi.org/10.1038/s41467-020-17612-8",
            "mutantMonocultureGrowth": 0.0,
            "mutantCocultureEcGrowth": 0.077574047954866,
            "mutantCocultureSeGrowth": 0.0348484100758386,
            "PHEtex_D": 0.0143717559943752,
            "PHELtex_A": -0.0143717559943752,
            "note": "References describe the deposited stationary gene-deletion predictions; WT reference-medium FBA is an explicitly added comparison, not an experimental validation.",
        },
    }
    published_errors = {
        "mutantMonocultureGrowth": abs(results[1]["objective"]),
        "mutantCocultureEcGrowth": abs(results[2]["growthObjectiveReactions"]["BIOMASS_Ec_iJO1366_core_53p95M_D"] - 0.077574047954866),
        "mutantCocultureSeGrowth": abs(results[2]["growthObjectiveReactions"]["BOF_A"] - 0.0348484100758386),
        "PHEtex_D": abs(results[2]["selectedReactions"]["PHEtex_D"]["flux"] - 0.0143717559943752),
        "PHELtex_A": abs(results[2]["selectedReactions"]["PHELtex_A"]["flux"] + 0.0143717559943752),
    }
    report["publishedReferenceValidation"] = {
        "absoluteErrors": published_errors, "tolerance": 1e-8,
        "passed": all(error < 1e-8 for error in published_errors.values()),
    }
    if not report["publishedReferenceValidation"]["passed"]:
        raise AssertionError("Calculated mutant predictions differ from Supplementary Data 9")
    if ko_path.exists():
        ko = io.loadmat(ko_path, simplify_cells=True)["modelCo2"]
        report["inputs"].setdefault(ko_path.name, {"sha256": sha256(ko_path)})
        wt_path = args.source_dir / "CM-models.mat"
        if wt_path.exists():
            wt = io.loadmat(wt_path, simplify_cells=True)["modelCo2"]
            report["inputs"][wt_path.name] = {"sha256": sha256(wt_path)}
            report["archiveKnockoutComparison"] = compare_knockout_archive(wt, ko)
    (args.output_dir / "validation.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    for result in results:
        print(f"{result['id']}: objective={result['objective']:.12g} growth={result['growthObjectiveReactions']} KO={result['modifiedBounds']}")
    print(f"Report: {args.output_dir / 'validation.json'}")


if __name__ == "__main__":
    main()
