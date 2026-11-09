"""Compare the Coxeter-MFV and COBRApy pFBA solutions of Figure 2.

pFBA has a unique optimal total flux but not always unique fluxes, so the
comparison reports both the agreement in growth and total flux and how many
reactions differ, over the whole model and over the 115 reactions on the map.
It also tallies, per compartment, the flux the map shows in each condition.

    python -I compare_fluxes.py
"""
import csv
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
RES = HERE / "results"
TOL = 1e-6


def read(name):
    with open(RES / name) as handle:
        return {row["reaction"]: float(row["flux"]) for row in csv.DictReader(handle)}


model = json.loads((HERE / "models" / "yeastgem_v9.1.1-edited.json").read_text())
names = {r["id"]: r.get("name", "") for r in model["reactions"]}
met_comp = {m["id"]: m.get("compartment") for m in model["metabolites"]}
rxn_comps = {r["id"]: sorted({met_comp[m] for m in r["metabolites"]}) for r in model["reactions"]}
map_doc = json.loads((HERE / "models" / "yeast9_cytosol_mitochondria_peroxisome.json").read_text())


def drawn_ids(doc):
    # Map documents keep their reactions under map.reactions, keyed by drawn id,
    # each pointing at its model reaction through bigg_id or id.
    reactions = (doc.get("map") or doc).get("reactions", {})
    items = reactions.values() if isinstance(reactions, dict) else reactions
    out = set()
    for entry in items:
        rid = entry.get("localId") or entry.get("id")
        if rid:
            out.add(rid.split("@")[-1])
    return out


drawn = drawn_ids(map_doc)
report = {"tolerance": TOL, "drawn_reactions_found": len(drawn)}
for condition in ("aerobic", "anaerobic"):
    cox = read(f"coxeter_pfba_{condition}.csv")
    ref = read(f"cobrapy_pfba_{condition}.csv")
    shared = set(cox) & set(ref)
    diff_all = sorted((abs(cox[r] - ref[r]), r) for r in shared if abs(cox[r] - ref[r]) > TOL)
    diff_map = [(d, r) for d, r in diff_all if r in drawn]
    active_map = {r: cox[r] for r in drawn if r in cox and abs(cox[r]) > TOL}
    by_compartment = {}
    for r, v in active_map.items():
        key = "+".join(rxn_comps[r]) if len(rxn_comps[r]) > 1 else rxn_comps[r][0]
        by_compartment.setdefault(key, []).append(r)
    report[condition] = {
        "reactions_compared": len(shared),
        "growth_coxeter": cox["r_2111"],
        "growth_cobrapy": ref["r_2111"],
        "total_flux_coxeter": sum(abs(cox[r]) for r in shared),
        "total_flux_cobrapy": sum(abs(ref[r]) for r in shared),
        "reactions_differing": len(diff_all),
        "largest_differences": [[r, names.get(r), cox[r], ref[r]] for d, r in diff_all[-10:]],
        "drawn_reactions_differing": len(diff_map),
        "drawn_differences": [[r, names.get(r), cox[r], ref[r]] for d, r in diff_map],
        "drawn_reactions_with_flux": len(active_map),
        "drawn_with_flux_by_compartment": {k: len(v) for k, v in sorted(by_compartment.items())},
        "drawn_fluxes": {r: [names.get(r), "+".join(rxn_comps[r]), v] for r, v in sorted(active_map.items())},
    }

(RES / "comparison.json").write_text(json.dumps(report, indent=1))
for condition in ("aerobic", "anaerobic"):
    c = report[condition]
    print(condition, {k: c[k] for k in ("growth_coxeter", "growth_cobrapy", "total_flux_coxeter",
          "total_flux_cobrapy", "reactions_differing", "drawn_reactions_differing",
          "drawn_reactions_with_flux", "drawn_with_flux_by_compartment")})
    for row in c["drawn_differences"]:
        print("   map diff", row)
print("drawn", report["drawn_reactions_found"])
