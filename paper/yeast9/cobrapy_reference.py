"""Independent reference for the Yeast9 aerobic and anaerobic pFBA of Figure 2.

Reads the yeast-GEM 9.1.1 SBML that Coxeter-MFV pins, makes the anaerobic
model with yeast-GEM's own anaerobic_model.py (commit 2d594ae, kept in
yeast-gem-2d594ae/), and solves FBA and pFBA for both conditions with COBRApy
and GLPK. It also writes, for every reaction the anaerobic conversion changes,
the new coefficients and bounds, so build_anaerobic_model.py can apply exactly
those changes to the Coxeter-MFV model JSON.

    python -I cobrapy_reference.py <yeast-GEM.xml> <output dir>
"""
import csv
import importlib.util
import json
import sys
from pathlib import Path

import cobra
from cobra.flux_analysis import pfba

HERE = Path(__file__).resolve().parent
SBML, OUT = Path(sys.argv[1]), Path(sys.argv[2])
OUT.mkdir(parents=True, exist_ok=True)

spec = importlib.util.spec_from_file_location(
    "anaerobic_model", HERE / "yeast-gem-2d594ae" / "code" / "anaerobic_model.py")
anaerobic = importlib.util.module_from_spec(spec)
spec.loader.exec_module(anaerobic)


def reaction_record(reaction):
    return {
        "lower_bound": reaction.lower_bound,
        "upper_bound": reaction.upper_bound,
        "metabolites": {m.id: c for m, c in reaction.metabolites.items()},
    }


def solve(model, label):
    model.solver = "glpk"
    model.solver.configuration.tolerances.feasibility = 1e-9
    fba = model.optimize()
    solution = pfba(model, fraction_of_optimum=1.0)
    fluxes = solution.fluxes
    with open(OUT / f"cobrapy_pfba_{label}.csv", "w", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerow(["reaction", "flux"])
        for rid, value in fluxes.items():
            writer.writerow([rid, repr(float(value))])
    return {
        "status": fba.status,
        "growth_fba": fba.objective_value,
        "growth_pfba": float(fluxes["r_2111"]),
        "total_absolute_flux": float(fluxes.abs().sum()),
        # Every exchange that carries flux, named, so the two conditions can be
        # compared without choosing the metabolites in advance.
        "exchanges": {
            r.id: {"name": r.name, "flux": float(fluxes[r.id])}
            for r in model.exchanges
            if abs(fluxes[r.id]) > 1e-9
        },
    }


aerobic_model = cobra.io.read_sbml_model(str(SBML))
before = {r.id: reaction_record(r) for r in aerobic_model.reactions}
anaerobic_model = aerobic_model.copy()
anaerobic.anaerobic_model(anaerobic_model)
after = {r.id: reaction_record(r) for r in anaerobic_model.reactions}

changes = {}
for rid, record in after.items():
    old = before[rid]
    if (record["lower_bound"] != old["lower_bound"]
            or record["upper_bound"] != old["upper_bound"]
            or record["metabolites"] != old["metabolites"]):
        changes[rid] = record
(OUT / "anaerobic_changes.json").write_text(json.dumps(changes, indent=1, sort_keys=True))

summary = {
    "cobra": cobra.__version__,
    "solver": "glpk",
    "sbml": str(SBML.name),
    "changed_reactions": sorted(changes),
    "aerobic": solve(aerobic_model, "aerobic"),
    "anaerobic": solve(anaerobic_model, "anaerobic"),
}
(OUT / "cobrapy_summary.json").write_text(json.dumps(summary, indent=1))
print(json.dumps(summary, indent=1))
