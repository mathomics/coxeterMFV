"""Phase 7: B. adolescentis' characteristic route, drawn although it carries no flux.

Ba is absent from the community at mu = 0.079 (X = 0), so phases 5 and 6, which
followed the flux, left its cell empty. Rule R18 asks for a species'
characteristic pathway as its GEM encodes it: for a bifidobacterium, the
fructose-6-phosphate shunt ("bifid shunt").

Anchors: three transporters of the GEM on the cytoplasmic membrane, with their
extracellular metabolites in the envelope band: the glucose PTS (GLCabcpp, top),
the acetate exporter (ACt2rpp, left) and the lactate exporter (LACt2rpp,
bottom). Ba's exchanges with the medium are not drawn: with X = 0 their
community coefficients are zero.

The route is drawn from a plan, as St's was:

* glucose PTS -> glucose 6-phosphate -> fructose 6-phosphate, down from the top;
* the shunt along the top row: F6P phosphoketolase (F6PP) gives acetyl
  phosphate and erythrose 4-phosphate; transaldolase, transketolase,
  ribose-5-phosphate isomerase and ribulose-5-phosphate epimerase carry the
  sugar phosphates on to xylulose 5-phosphate;
* acetyl phosphate -> acetate (ACKr), out through the acetate exporter;
* glyceraldehyde 3-phosphate -> lactate, down a column to the lactate exporter.

Real gaps of the GEM are not bridged (R14, R24) and are written in the log:
there is no xylulose-5-phosphate phosphoketolase (the shunt's second
phosphoketolase) and no phosphofructokinase, which is why bifidobacteria use
the shunt at all.

Usage: python build_phase7_ba.py IN OUT LOG
"""
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from build_phase5_transport import main_inside, species  # noqa: E402
from build_phase7_substrate_first import LABEL_SIZE, build  # noqa: E402
from coxmap import L_REF, Builder, CoxMap, load, save, write_log  # noqa: E402

CELL = 'Ba'
# transporter, membrane edge, position along it
TRANSPORTERS = [('GLCabcpp_Ba', 'top', 21300), ('ACt2rpp_Ba', 'left', 20524), ('LACt2rpp_Ba', 'bottom', 21860)]
OUTSIDE = 190     # the extracellular node, this far out from the cytoplasmic membrane (the band is 340 wide)

PLAN = {
    'nodes': {
        'g6p': 'T:g6p', 'f6p': ('f6p', 21300, 19964),
        # the shunt, along the top row
        'e4p': ('e4p', 21580, 19964), 's7p': ('s7p', 21860, 19964), 'r5p': ('r5p', 22140, 19964),
        'ru5p': ('ru5p_D', 22420, 19964), 'xu5p': ('xu5p_D', 22700, 19964),
        'actp': ('actp', 21300, 20244), 'ac': 'T:ac',
        # glyceraldehyde 3-phosphate to lactate, down to the lactate exporter
        'g3p': ('g3p', 21860, 21436), '13dpg': ('13dpg', 21860, 21716), '3pg': ('3pg', 21860, 21996),
        '2pg': ('2pg', 21860, 22276), 'pep': ('pep', 21860, 22556), 'pyr': ('pyr', 21860, 22836),
        'lac': 'T:lac_L',
    },
    'reactions': [
        ('PGI', 'g6p', 'f6p'),
        ('F6PP_bifido_shunt', 'f6p', 'e4p', {'actp': 'actp'}),
        ('ACKr', 'actp', 'ac'),
        ('TALA', 'e4p', 's7p'), ('TKT1', 's7p', 'r5p'), ('RPI', 'r5p', 'ru5p'), ('RPE', 'ru5p', 'xu5p'),
        ('GAPD', 'g3p', '13dpg'), ('PGK', '13dpg', '3pg'), ('PGM', '3pg', '2pg'), ('ENO', '2pg', 'pep'),
        ('PYK', 'pep', 'pyr'), ('LDH_L', 'pyr', 'lac'),
    ],
    'labels': [
        ('Bifid shunt', 21900, 20420),
        ('Glycolysis (lower part)', 22050, 22080),
    ],
}

GAPS = [
    'no xylulose-5-phosphate phosphoketolase: xylulose 5-phosphate does not give acetyl phosphate and '
    'glyceraldehyde 3-phosphate directly; the shunt is incomplete there',
    'no phosphofructokinase: fructose 6-phosphate does not go on to fructose 1,6-bisphosphate, which is '
    'why the shunt carries the hexose',
]


def outward(edge):
    return {'top': (0, -1), 'bottom': (0, 1), 'left': (-1, 0), 'right': (1, 0)}[edge]


def add_transporter(cm, s, b, rid, edge, along):
    gem = s.rxns[rid]
    out_loc = next(k for k in gem['metabolites'] if s.comp(k) == 'e' and not s.is_currency(k))
    in_loc = main_inside(s, rid, out_loc, 'c')
    mx, my = b.membrane_point('c', edge, along)
    ox, oy = outward(edge)
    mark = cm.mark()
    e_node = b.node(out_loc, 'metabolite', mx + ox * OUTSIDE, my + oy * OUTSIDE, s.mid(rid))
    inner = b.node(in_loc, 'metabolite', mx - ox * L_REF, my - oy * L_REF, s.mid(rid))
    res = b.add_transport(rid, 'c', 'e', edge, along, {out_loc: e_node['id']}, {in_loc: inner['id']},
                          note='phase 7: anchor of the bifid shunt')
    if res is None or res:
        cm.rollback(mark)
        return res or ['no room for its cofactors']
    return []


def main(src, out, log_path):
    cm = CoxMap(load(src))
    s = species(CELL)
    log = []
    b = Builder(cm, s, log)
    notes = []
    for rid, edge, along in TRANSPORTERS:
        problems = add_transporter(cm, s, b, rid, edge, along)
        if problems:
            notes.append(f'{rid}: not drawn ({"; ".join(problems[:2])})')
    results = build(cm, s, PLAN, log)
    notes += [f'{r}: not drawn ({"; ".join(p[:2])})' for r, p in results if p]
    texts = cm.M.setdefault('texts', [])
    texts[:] = [t for t in texts if not t['id'].startswith(f'pathway-{CELL}-')]
    for text, x, y in PLAN['labels']:
        slug = ''.join(ch if ch.isalnum() else '-' for ch in text.lower()).strip('-')
        texts.append({'id': f'pathway-{CELL}-{slug}', 'text': text, 'x': x, 'y': y,
                      'fontSize': LABEL_SIZE, 'fontFamily': 'sans'})
    save(cm.doc, out)
    drawn = sum(1 for r, p in results if not p)
    report = f'{CELL}: {len(TRANSPORTERS)} transporters and {drawn} of {len(results)} planned reactions'
    print(report)
    for x in notes:
        print('  ', x)
    write_log(log_path, 'Fase 7: la vía característica de Ba', 'Ba no tiene flujo a mu = 0,079 (X = 0); se dibujó su '
              'vía característica, el bifid shunt, como la codifica su GEM (R18, R24), desde un plano.', log,
              closing='\n'.join('- ' + x for x in [report] + notes + ['Discontinuidad del GEM: ' + g for g in GAPS]),
              append=Path(log_path).exists())


if __name__ == '__main__':
    main(*sys.argv[1:4])
