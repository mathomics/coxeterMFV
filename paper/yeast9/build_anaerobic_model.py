"""Write the anaerobic Yeast9 model that opens on the same map as the aerobic one.

Starts from the Coxeter-MFV example model (yeastgem_v9.1.1-edited.json),
first checks that it holds the same reactions, coefficients, bounds and
objective as the pinned yeast-GEM 9.1.1 SBML, then applies the reaction
changes that yeast-GEM's anaerobic_model.py made in cobrapy_reference.py
(results/anaerobic_changes.json). Nothing else in the model changes, so the
map's reaction and metabolite ids stay valid.

    python -I build_anaerobic_model.py <yeast-GEM.xml> <edited model json> <output json>
"""
import copy
import json
import sys
from pathlib import Path

import cobra

HERE = Path(__file__).resolve().parent
SBML, EDITED, OUT = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])

sbml = cobra.io.read_sbml_model(str(SBML))
doc = json.loads(EDITED.read_text())
changes = json.loads((HERE / "results" / "anaerobic_changes.json").read_text())

# The example model must be the SBML, reaction for reaction, before anything
# is applied to it.
mismatches = []
by_id = {r["id"]: r for r in doc["reactions"]}
if set(by_id) != {r.id for r in sbml.reactions}:
    mismatches.append("reaction ids differ")
for reaction in sbml.reactions:
    entry = by_id.get(reaction.id)
    if entry is None:
        continue
    expected = {m.id: c for m, c in reaction.metabolites.items()}
    if set(entry["metabolites"]) != set(expected) or any(
            abs(entry["metabolites"][m] - c) > 1e-12 for m, c in expected.items()):
        mismatches.append(f"{reaction.id} coefficients")
    if (entry["lower_bound"], entry["upper_bound"]) != (reaction.lower_bound, reaction.upper_bound):
        mismatches.append(f"{reaction.id} bounds")
    if (entry.get("objective_coefficient") or 0) != reaction.objective_coefficient:
        mismatches.append(f"{reaction.id} objective")
if {m["id"] for m in doc["metabolites"]} != {m.id for m in sbml.metabolites}:
    mismatches.append("metabolite ids differ")
if mismatches:
    sys.exit("The example model is not the SBML: " + ", ".join(mismatches[:20]))

anaerobic = copy.deepcopy(doc)
# The name stays, since the map labels the cell with it. The id says which
# condition this is.
anaerobic["id"] = doc["id"] + "_anaerobic"
anaerobic.setdefault("notes", {})["condition"] = (
    "Anaerobic, made with yeast-GEM anaerobic_model.py at commit 2d594ae")
applied = []
for entry in anaerobic["reactions"]:
    change = changes.get(entry["id"])
    if change is None:
        continue
    entry["lower_bound"] = change["lower_bound"]
    entry["upper_bound"] = change["upper_bound"]
    entry["metabolites"] = dict(change["metabolites"])
    applied.append(entry["id"])
if sorted(applied) != sorted(changes):
    sys.exit("Not every anaerobic change found its reaction")

OUT.write_text(json.dumps(anaerobic))
print(json.dumps({
    "example_model_matches_sbml": True,
    "reactions": len(doc["reactions"]),
    "metabolites": len(doc["metabolites"]),
    "anaerobic_changes_applied": sorted(applied),
    "output": OUT.name,
}, indent=1))
