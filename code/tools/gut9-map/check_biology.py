"""What a gut9 SteadyCom solution says about each species, against what is known of it.

Reads one or more flux CSVs exported from Coxeter (per gram of each species; the
community exchanges EX_biomassK(u) give the abundances, X_k = EX_biomassK(u) / mu)
and prints a markdown table, one column per CSV: composition, short-chain fatty
acids, the fermentation phenotype of each member that the literature describes
(butyrate from acetate in Fp, no lactate use in Er or Bt, lactate from the lactic
acid bacteria), oxygen released in an anaerobic community, fluxes at the
±1000 bound, total community flux and two loops between members seen before.
Then the main exchanges of each present member.

Usage: python tools/gut9-map/check_biology.py LABEL=path.csv [LABEL=path.csv ...]
"""
import csv
import json
import sys
from collections import defaultdict
from pathlib import Path

MODELS = Path(__file__).resolve().parents[2] / 'models' / 'published' / 'steadycom-gut-nine'
SPECIES = ['Bt', 'Fp', 'Kp', 'St', 'Ba', 'Ec', 'Ef', 'Er', 'Lc']
TOL = 1e-6


def exchange_map():
    """Species exchange reaction -> (species, community metabolite): the reactions with x[u] at +1."""
    out = {}
    for sp in SPECIES:
        model = json.loads((MODELS / f'model-{sp}.json').read_text(encoding='utf-8'))
        for r in model['reactions']:
            u = [k for k in r['metabolites'] if k.endswith('[u]')]
            if len(u) == 1 and r['metabolites'][u[0]] == 1:
                out[f'{sp}@{r["id"]}'] = (sp, u[0][:-3])
    return out


EXCHANGES = exchange_map()


def load(path):
    flux = {row['reaction']: float(row['flux']) for row in csv.DictReader(open(path, encoding='utf-8'))}
    mu = max(v for k, v in flux.items() if '@EX_biomass' in k)
    X = {sp: flux.get(f'EX_biomass{sp}(u)', 0.0) / mu for sp in SPECIES}
    ex = defaultdict(dict)
    for k, (sp, base) in EXCHANGES.items():
        v = flux.get(k, 0.0)
        if abs(v) > TOL and X[sp] > 1e-9:
            ex[sp][base] = v
    return {'flux': flux, 'mu': mu, 'X': X, 'ex': ex}


def community(d, sp, base):
    return d['X'][sp] * d['ex'][sp].get(base, 0.0)


def produced(d, base):
    return sum(max(community(d, sp, base), 0) for sp in SPECIES)


def consumed(d, base):
    return sum(max(-community(d, sp, base), 0) for sp in SPECIES)


def pct(x):
    if x <= 1e-9:
        return '0'
    if x < 5e-5:
        return '< 0.01 %'
    return f'{100 * x:.1f} %' if x >= 0.001 else f'{100 * x:.2f} %'


def checklist(d):
    X, ex = d['X'], d['ex']
    rows = [('Composición', ', '.join(f'{k} {pct(v)}' for k, v in sorted(X.items(), key=lambda kv: -kv[1]) if v > 5e-5)),
            ('μ (h⁻¹)', f'{d["mu"]:.5f}'),
            ('St + Ef', pct(X['St'] + X['Ef'])),
            ('Bt + Fp + Er', pct(X['Bt'] + X['Fp'] + X['Er'])),
            ('Ba presente', f'sí ({pct(X["Ba"])})' if X['Ba'] > 1e-6 else 'no')]
    ac = max(produced(d, 'ac') - consumed(d, 'ac'), 0)
    ppa, but = produced(d, 'ppa'), produced(d, 'but')
    total = ac + ppa + but
    rows.append(('Acetato:propionato:butirato (neto)',
                 f'{100 * ac / total:.0f} : {100 * ppa / total:.0f} : {100 * but / total:.0f}' if total > 0 else '—'))
    rows.append(('Productores de butirato (mmol·h⁻¹)',
                 ', '.join(f'{sp} {community(d, sp, "but"):.3f}' for sp in SPECIES if community(d, sp, 'but') > 1e-5) or 'ninguno'))
    rows.append(('Fp: acetato → butirato (/gDW)',
                 f'acetato {-ex["Fp"].get("ac", 0):.3g}, butirato {ex["Fp"].get("but", 0):.3g}' if X['Fp'] > 1e-9 else 'ausente'))
    for sp in ['Er', 'Bt']:
        lactate = -ex[sp].get('lac-L', 0) - ex[sp].get('lac-D', 0)
        rows.append((f'{sp} usa lactato (no debería, /gDW)',
                     (f'{lactate:.3g}' if lactate > 1e-6 else 'no') if X[sp] > 1e-9 else 'ausente'))
    for sp in ['St', 'Ef', 'Lc', 'Ba']:
        if X[sp] > 1e-9:
            lactate = ex[sp].get('lac-L', 0) + ex[sp].get('lac-D', 0)
            rows.append((f'{sp} produce lactato (/gDW)', f'{lactate:.3g}' if lactate > 1e-6 else 'no'))
    o2 = [(sp, ex[sp].get('o2', 0)) for sp in SPECIES if ex[sp].get('o2', 0) > 1e-6]
    rows.append(('Alguna especie libera O₂ (/gDW)', ', '.join(f'{sp} {v:.3g}' for sp, v in o2) or 'no'))
    rows.append(('H₂ liberado (mmol·h⁻¹)', f'{produced(d, "h2"):.3f}'))
    at_bound = defaultdict(int)
    for k, v in d['flux'].items():
        if '@' in k and abs(v) >= 999:
            at_bound[k.split('@')[0]] += 1
    rows.append(('Flujos ≥ 999 /gDW', ', '.join(f'{sp} {n}' for sp, n in sorted(at_bound.items())) or 'ninguno'))
    rows.append(('Flujo total Σ X·|v|', f'{sum(X[k.split("@")[0]] * abs(v) for k, v in d["flux"].items() if "@" in k):.1f}'))
    rows.append(('Ciclo glutamato/citrato', f'{min(produced(d, "cit"), consumed(d, "cit")):.3f}'))
    rows.append(('Ciclo lactato/piruvato', f'{min(produced(d, "pyr"), consumed(d, "pyr")):.3f}'))
    return rows


def exchanges(d):
    lines = []
    for sp in SPECIES:
        if d['X'][sp] <= 1e-9:
            lines.append(f'- {sp}: ausente')
            continue
        e = d['ex'][sp]
        size = lambda kv: -abs(kv[1])
        taken = sorted(((b, v) for b, v in e.items() if v < 0 and b not in ('h', 'h2o')), key=size)[:8]
        given = sorted(((b, v) for b, v in e.items() if v > 0 and b not in ('h', 'h2o') and not b.startswith('biomass')), key=size)[:8]
        lines.append(f'- {sp} ({pct(d["X"][sp])}): toma ' + ', '.join(f'{b} {-v:.3g}' for b, v in taken)
                     + ' | libera ' + ', '.join(f'{b} {v:.3g}' for b, v in given))
    return lines


def main(args):
    runs = [(label, load(path)) for label, path in (a.rsplit('=', 1) for a in args)]
    tables = [checklist(d) for _, d in runs]
    out = ['| Criterio | ' + ' | '.join(label for label, _ in runs) + ' |', '|---' * (len(runs) + 1) + '|']
    for i, (name, _) in enumerate(tables[0]):
        out.append(f'| {name} | ' + ' | '.join(t[i][1] for t in tables) + ' |')
    for label, d in runs:
        out += ['', f'**{label}** (por gramo de cada especie):', ''] + exchanges(d)
    sys.stdout.buffer.write(('\n'.join(out) + '\n').encode('utf-8'))


if __name__ == '__main__':
    main(sys.argv[1:])
