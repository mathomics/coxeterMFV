"""Phase 3c of the gut9 map: where Bt's acetyl-CoA comes from at mu = 0.079.

In this scenario PFL and POR4i carry no flux. Bt's acetyl-CoA is made from
threonine: taken up (THRt2r) and also rebuilt from glycine and acetaldehyde
(THRAr, run backwards), it is oxidised to 2-amino-3-oxobutanoate (THRD), which
GLYAT splits with CoA into acetyl-CoA and glycine; glycine leaves the cell
(GLYt2r). Five reactions, added one at a time and each checked against the GEM:

  THRt2r, THRD, GLYAT, GLYt2r, THRAr

The module sits above the acetate branch, between acetyl-CoA and the right
membrane, where the threonine and glycine exchanges are moved. ACKr's cofactors
go below its axis to make room. Acetaldehyde is drawn as a primary node beside
THRAr; the reaction that makes it (ALCD2x, from ethanol) is not drawn yet and
is recorded as an open end (R14).

Usage: python build_phase3c.py MAP MODEL_JSON FLUXES_CSV OUT_MAP LOG_MD
"""
import csv
import json
import math
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from build_phase3b import TRANSPORT_GAP, Builder, mid, r2  # noqa: E402


def write_log(b, S, path, title, intro, closing):
    with open(path, 'w', encoding='utf-8') as f:
        f.write(f'# {title}\n\n{intro} L_ref = {S:.1f}. Flujos del escenario a μ = 0,078998.\n\n')
        f.write('| # | Reacción | Ecuación del GEM | Bounds | Flujo | Nodos principales | Tramo máx. | Nota |\n'
                '|---|---|---|---|---|---|---|---|\n')
        for i, e in enumerate(b.log, 1):
            f.write(f'| {i} | {e["reaction"]} | `{e["equation"]}` | {e["bounds"]} | {e["flux"]:.4g} | {e["nodes"]} | '
                    f'{e["max_main"]:.0f} | {e["note"]} |\n')
        f.write(f'\n{closing}\n')


def main(map_path, model_path, flux_path, out_path, log_path):
    doc = json.load(open(map_path, encoding='utf-8'))
    model = json.load(open(model_path, encoding='utf-8'))
    flux = {r['local_id']: float(r['flux']) for r in csv.DictReader(open(flux_path, encoding='utf-8')) if r['species'] == 'Bt'}
    b = Builder(doc, model, flux)
    N = b.N

    chain = ['met:Bt@PGI_Bt:Bt@g6p[c_Bt]', 'met:Bt@PGI_Bt:Bt@f6p[c_Bt]', 'met:Bt@PFK_Bt:Bt@fdp[c_Bt]',
             'met:Bt@FBA_Bt:Bt@g3p[c_Bt]', 'met:Bt@GAPD_Bt:Bt@13dpg[c_Bt]', 'met:Bt@PGK_Bt:Bt@3pg[c_Bt]']
    S = statistics.median(math.hypot(N[p]['x'] - N[q]['x'], N[p]['y'] - N[q]['y']) for p, q in zip(chain, chain[1:]))
    P = N['met:Bt@PYK_Bt:Bt@pyr[c_Bt]']
    ACCOA = 'met:Bt@PFL_Bt:Bt@accoa[c_Bt]'
    at = lambda dx, dy: (P['x'] + dx * S, P['y'] + dy * S)
    ext = lambda m: f'met:Bt@EX_{m}[u]tr_Bt:Bt@{m}[e_Bt]'

    # room above the acetate branch: ACKr's cofactors go under its axis
    b.relayout_cofactors('ACKr_Bt', hands={'atp[c_Bt]': -1, 'adp[c_Bt]': -1})

    # glycine and threonine rows one transporter gap and two above the acetate row
    g = TRANSPORT_GAP
    T, B, G = at(3.2, -2 * g), at(2.2, -1.07 * g), at(3.2, -g)
    ER = b.cell['rect']['x'] + b.cell['rect']['w']
    b.move_exchange('EX_thr-L[u]tr_Bt', ER, T[1], (1, 0))
    b.move_exchange('EX_gly[u]tr_Bt', ER, G[1], (1, 0))

    thr = b.node('met:Bt@THRt2r_Bt:Bt@thr-L[c_Bt]', 'thr-L[c_Bt]', 'metabolite', *T, rid='Bt@THRt2r_Bt')
    b.add_symport('THRt2r_Bt', thr['id'], ext('thr-L'), (1, 0))
    b.record('THRt2r_Bt', 'creates thr-L[c]; reuses thr-L[e] (EX_thr-L, moved to the right edge)')

    aob = b.node('met:Bt@THRD_Bt:Bt@2aobut[c_Bt]', '2aobut[c_Bt]', 'metabolite', *B, rid='Bt@THRD_Bt')
    b.add_metabolic('THRD_Bt', thr['id'], aob['id'])
    b.record('THRD_Bt', 'reuses thr-L[c]; creates 2aobut[c]')

    gly = b.node('met:Bt@GLYAT_Bt:Bt@gly[c_Bt]', 'gly[c_Bt]', 'metabolite', *G, rid='Bt@GLYAT_Bt')
    b.add_metabolic('GLYAT_Bt', ACCOA, aob['id'], primaries={gly['mergedId']: gly['id']})
    b.record('GLYAT_Bt', 'reuses accoa[c] (PFL/POR4i/PTAr) and 2aobut[c]; creates gly[c]')

    b.add_symport('GLYt2r_Bt', gly['id'], ext('gly'), (1, 0))
    b.record('GLYt2r_Bt', 'reuses gly[c] and gly[e] (EX_gly, moved to the right edge)')

    # acetaldehyde between the threonine and glycine exits, clear of both
    acald = b.node('met:Bt@THRAr_Bt:Bt@acald[c_Bt]', 'acald[c_Bt]', 'metabolite', *at(3.75, -1.5 * g), rid='Bt@THRAr_Bt')
    b.add_metabolic('THRAr_Bt', thr['id'], gly['id'], primaries={acald['mergedId']: acald['id']})
    b.record('THRAr_Bt', 'reuses thr-L[c] and gly[c]; creates acald[c], whose source (ALCD2x, from ethanol) is not drawn yet')
    b.tidy(['PYK_Bt', 'LDH_L_Bt', 'LDH_D_Bt', 'PFL_Bt', 'POR4i_Bt', 'PTAr_Bt', 'ACKr_Bt', 'THRD_Bt', 'GLYAT_Bt', 'THRAr_Bt'])

    md = b.doc['metadata']
    md['reactionCount'], md['nodeCount'], md['segmentCount'] = len(b.M['reactions']), len(b.M['nodes']), len(b.M['segments'])
    json.dump(b.doc, open(out_path, 'w', encoding='utf-8'), ensure_ascii=False, indent=1)
    write_log(b, S, log_path, 'Fase 3c de Bt — registro de inserciones',
              'Generado por `tools/gut9-map/build_phase3c.py`.',
              'Cada inserción pasó la verificación de R51 antes de la siguiente. Discontinuidad registrada (R14): '
              'el acetaldehído de THRAr proviene de ALCD2x (etanol → acetaldehído, 1,10), que no está dibujado todavía.')
    print(f'L_ref {S:.1f}; {len(b.log)} reactions added')
    for e in b.log:
        print(f'  {e["reaction"]:12} max main branch {e["max_main"]:6.0f}  ok')


if __name__ == '__main__':
    main(*sys.argv[1:6])
