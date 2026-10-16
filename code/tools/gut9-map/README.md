# gut9-map

Scripts that build and check the curated map of the nine-species gut community
(`models/published/steadycom-gut-nine/map.json`). The history, decisions and
lessons are in [docs/gut9-map/GUT9_MAPA_BITACORA.md](../../docs/gut9-map/GUT9_MAPA_BITACORA.md);
the construction rules in [docs/gut9-map/COXETER_REGLAS_CONSTRUCCION_MAPAS.md](../../docs/gut9-map/COXETER_REGLAS_CONSTRUCCION_MAPAS.md).

They need Python 3.7+ and numpy (`build_phase2b.py`). Run them from the
repository root.

| Script | What it does |
|---|---|
| `coxmap.py` | Library the later phases use: the models' four namespaces reduced to one base, cofactor detection, nodes, reactions and transporters in Coxeter's format, the R51 check of each insertion against its GEM, crossings and clearance with a spatial index, and rollback. |
| `build_phase3a_v2.py` | Redraws the phase 3a map at the scale of the hand-drawn map: copies the manual reactions as they are, lays the new ones out at L_ref, and resizes bands and exchanges. Geometry only. |
| `build_phase2b.py` | Places the species on the 3x3 grid (all 9! assignments scored by cross-feeding weight x distance, crossings and links through other cells) and turns each exchange towards its partners. Flows are X_k·v_k. Geometry only. |
| `build_phase3b.py` | Adds Bt's pyruvate outlets and acetate branch (9 reactions), one at a time, each checked against the GEM; moves the glycolysis module as a block next to the exchanges it feeds, and writes an insertion log. |
| `build_phase3c.py` | Adds where Bt's acetyl-CoA comes from at mu = 0.079: threonine and glycine (THRt2r, THRD, GLYAT, GLYt2r, THRAr), reusing the phase 3b builder. |
| `build_phase4_resize.py` | Shrinks the cells to about 6000 x 5500 on the same grid, moves content as blocks with its corner, and assigns the other exchanges around each perimeter, facing their partners and keeping their order. |
| `build_phase5_transport.py` | The transporter layer of one species: for every exchange, the GEM reaction(s) with the largest flux that carry it into the cytosol (through the periplasm in Ec and Kp), in line with the exchange. |
| `build_phase6_grow.py` | Grows a species' internal network from what is drawn: active reactions by flux, new metabolites 1–1.5 L_ref away continuing their pathway along the axes, joins only when close and clear. |
| `phase6_driver.py` | Runs phase 6 in batches and commits each batch that passes the validator and the audit. |
| `build_phase7_substrate_first.py` | Takes a species' internal reactions out and draws its central metabolism again from a plan: each substrate's pathway starts at its transporter and runs inward; products are drawn where they are made, as duplicates of their exit's node; each pathway gets a text label. Plans so far: St (pilot). |
| `build_phase7_spines.py` | The same, automatic, for the other species: spines from every substrate taken up, following the most carbon flux through central metabolism, one step at a time straight inward (turning 90 degrees only when blocked); products end on their exit when it is close, otherwise as duplicates; biomass-precursor work (large molecules, acyl carriers) only as short branches at the end; one "Substrate -> Product" label per main pathway. Keeps the hand-drawn reactions of phases 3a-3c. |
| `pathways.py` | The conceptual pathways (glycolysis, PPP, fermentations, TCA, nitrogen, Leloir, maltodextrins, fructans, uronic acids, butyrate, propanediol), written once in BiGG names, and how to find each step in any of the nine GEMs by metabolite identity (ModelSEED ids through an explicit name table). |
| `build_phase7_pathways.py` | Completes those pathways in every species that uses them: the main route with every step the GEM has, active or not, first; then the active bypasses, as parallel arcs beside the step they replace; a traced copy of an intermediate when there is no room round it (logged); one label per pathway. |
| `build_phase7_ba.py` | Draws Ba's characteristic route, the bifid shunt, from a plan although it carries no flux: three GEM transporters as anchors, then the shunt, acetate and lower glycolysis to lactate; logs the GEM's real gaps (no xylulose-5-phosphate phosphoketolase, no phosphofructokinase). |
| `build_phase7_bt_band.py` | Widens the left side of Bt's envelope band outwards, moving the exchanges on that side with it, puts its fructan exchanges in pathway order and draws their extracellular hydrolysis. |
| `check_biology.py` | Reads flux CSVs exported from Coxeter and checks each solution against the biology of the nine species: composition, short-chain fatty acids, butyrate from acetate in Fp, no lactate use in Er or Bt, lactate from the lactic acid bacteria, oxygen released, fluxes at the bound, total flux and loops between members. One markdown column per CSV. |
| `validate_map.py` | Structural and geometric checks: compartments, overlaps, segment ends, lengths; with a reference map, also unchanged ids, equations, bounds and connectivity. |
| `audit.py` | Crossings and crowded nodes over the whole map, cell by cell (exit status 1 if any). |
| `crossings.py` | The same for one region, without the index (slower; kept for the phase 3 logs). |
| `crossfeeding_metrics.py` | Length and crossings of the donor–receiver links, to compare two layouts. |
| `publish.py` | Makes a map the community's default map, also inside `coxeter_mfv.html`, and optionally adds a log section and a snapshot. |

## Rebuilding the current map

Phases 2b–3c (each output is the next input):

```bash
python tools/gut9-map/build_phase3a_v2.py docs/gut9-map/maps/00-manual-base-map.json docs/gut9-map/maps/01-phase3a-Bt-glycolysis-rejected.json docs/gut9-map/maps/02-phase3a-v2-Bt-glycolysis.json
python tools/gut9-map/build_phase2b.py docs/gut9-map/maps/02-phase3a-v2-Bt-glycolysis.json docs/gut9-map/inputs/fluxes.csv docs/gut9-map/maps/04-phase2b-layout.json
python tools/gut9-map/build_phase3b.py docs/gut9-map/maps/04-phase2b-layout.json models/published/steadycom-gut-nine/model-Bt.json docs/gut9-map/inputs/fluxes.csv docs/gut9-map/maps/05-phase3b-Bt-pyruvate-acetate.json docs/gut9-map/FASE3B_BT_REGISTRO.md
python tools/gut9-map/build_phase3c.py docs/gut9-map/maps/05-phase3b-Bt-pyruvate-acetate.json models/published/steadycom-gut-nine/model-Bt.json docs/gut9-map/inputs/fluxes.csv out-3c.json docs/gut9-map/FASE3C_BT_REGISTRO.md
```

Phases 4–6 (snapshots `06-…`, `07-…`, `08-…` in `docs/gut9-map/maps/`):

```bash
python tools/gut9-map/build_phase4_resize.py out-3c.json docs/gut9-map/inputs/fluxes.csv out-4.json
python tools/gut9-map/build_phase5_transport.py out-4.json out-5-St.json St log5.md 40      # then Ef, Bt, Fp, Er, Lc, Ec (x2), Kp (x2)
python tools/gut9-map/phase6_driver.py St 70 10                                              # grows the published map in place, one commit per batch
```

Phase 7 (snapshot `09-…`):

```bash
python tools/gut9-map/build_phase7_substrate_first.py St docs/gut9-map/maps/08-phase6-1000-reactions.json docs/gut9-map/maps/09-phase7-St-pilot.json docs/gut9-map/FASE7_SUSTRATOS.md
python tools/gut9-map/build_phase7_spines.py docs/gut9-map/maps/09-phase7-St-pilot.json docs/gut9-map/maps/10-phase7-all-cells.json docs/gut9-map/FASE7_SUSTRATOS.md Ef:65 Bt:77 Fp:60 Ec:60 Kp:45 Er:25 Lc:25
python tools/gut9-map/build_phase7_pathways.py docs/gut9-map/maps/10-phase7-all-cells.json docs/gut9-map/maps/11-phase7-pathways.json docs/gut9-map/FASE7_SUSTRATOS.md St Ef Bt Fp Ec Kp Er Lc
python tools/gut9-map/build_phase7_ba.py docs/gut9-map/maps/11-phase7-pathways.json step-ba.json docs/gut9-map/FASE7_SUSTRATOS.md
python tools/gut9-map/build_phase7_bt_band.py step-ba.json docs/gut9-map/maps/12-phase7-Ba-Bt-envelope.json docs/gut9-map/FASE7_SUSTRATOS.md
```

Checks:

```bash
python tools/gut9-map/check_biology.py "Actual=docs/gut9-map/inputs/fluxes.csv" "Fibra 4=docs/gut9-map/inputs/fluxes-fibra4-parsimonioso.csv"
python tools/gut9-map/validate_map.py models/published/steadycom-gut-nine/map.json
python tools/gut9-map/audit.py models/published/steadycom-gut-nine/map.json
```

`build_phase2b.py` also writes `<out>.crossfeeding.json` (the pairs and weights);
the copy kept in the repository is `docs/gut9-map/maps/03-phase2b-crossfeeding.json`.

`publish.py` updates the map embedded in `coxeter_mfv.html`; with Node available,
`node tools/build-standalone.mjs` does the same and more.
