"""Compare Coxeter's mapped Yeast9 FVA intervals with independent COBRApy solves."""

import json
from pathlib import Path

import cobra
from cobra.flux_analysis import flux_variability_analysis, pfba

HERE = Path(__file__).resolve().parent
OUT = HERE / "results" / "solver-comparison"


def main():
    comparison = json.loads((OUT / "comparison.json").read_text(encoding="utf-8"))
    model = cobra.io.load_json_model(str(HERE / "models" / "yeastgem_v9.1.1-edited.json"))
    model.solver = "glpk"
    # Match the independent reference settings used for the manuscript's yeast analysis.
    model.solver.configuration.tolerances.feasibility = 1e-9
    ids = [row["id"] for row in comparison["rows"]]
    solution = pfba(model)
    report = {
        "cobraVersion": cobra.__version__,
        "solver": "GLPK",
        "feasibilityTolerance": 1e-9,
        "growth": float(solution.fluxes["r_2111"]),
        "totalAbsoluteFlux": float(solution.fluxes.abs().sum()),
        "objectiveFraction": 1.0,
    }
    for method in ["fva"]:
        ranges = flux_variability_analysis(
            model,
            reaction_list=ids,
            fraction_of_optimum=1.0,
            processes=1,
        )
        ranges.to_csv(OUT / f"cobrapy_{method}.csv", index_label="reaction")
        errors = []
        for row in comparison["rows"]:
            for key, column in [("min", "minimum"), ("max", "maximum")]:
                value = float(ranges.loc[row["id"], column])
                error = abs(row[method][key] - value)
                errors.append({"reaction": row["id"], "endpoint": key, "absoluteError": error})
        errors.sort(key=lambda row: row["absoluteError"], reverse=True)
        report[method] = {"endpoints": len(errors), "largestErrors": errors[:10]}
        print(method, report[method], flush=True)
    (OUT / "cobrapy_validation.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
