"""Check whether the map reactions where Coxeter-MFV and COBRApy differ can
take either value at the same minimum total flux (pFBA alternative optima).

FVA at 100 % of optimal growth with the total absolute flux held within
1.00001 times its minimum, for each drawn reaction that differed.

    python -I alternative_optima.py <yeast-GEM.xml>
"""
import importlib.util
import json
import sys
from pathlib import Path

import cobra
from cobra.flux_analysis import flux_variability_analysis

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    "anaerobic_model", HERE / "yeast-gem-2d594ae" / "code" / "anaerobic_model.py")
anaerobic = importlib.util.module_from_spec(spec)
spec.loader.exec_module(anaerobic)

comparison = json.loads((HERE / "results" / "comparison.json").read_text())
base = cobra.io.read_sbml_model(sys.argv[1])
out = {}
for condition in ("aerobic", "anaerobic"):
    model = base.copy()
    if condition == "anaerobic":
        anaerobic.anaerobic_model(model)
    model.solver = "glpk"
    ids = [row[0] for row in comparison[condition]["drawn_differences"]]
    if not ids:
        continue
    fva = flux_variability_analysis(model, reaction_list=ids, fraction_of_optimum=1.0, pfba_factor=1.00001)
    out[condition] = {
        rid: {"min": float(fva.loc[rid, "minimum"]), "max": float(fva.loc[rid, "maximum"]),
              "coxeter": row[2], "cobrapy": row[3]}
        for rid, row in zip(ids, comparison[condition]["drawn_differences"])
    }
(HERE / "results" / "alternative_optima.json").write_text(json.dumps(out, indent=1))
print(json.dumps(out, indent=1))
