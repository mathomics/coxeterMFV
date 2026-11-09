"""Conceptual pathways, written once in BiGG names, found in any of the nine GEMs.

A pathway is an ordered list of steps; a step is a substrate and a product
(BiGG bases, as coxmap.Species.base writes them) and the reaction ids that
usually carry it. A GEM "has" a step when one of its reactions turns that
substrate into that product, whatever the reaction is called: BiGG, Ef's
M_-prefixed ids and the ModelSEED cpd ids of Lc and Er are all matched by
metabolite identity, the ModelSEED ones through their names (R12: an explicit,
checked equivalence, never a guess from a partial name).

`alt=True` marks a bypass or alternative route (R19, R20): it is drawn only
after the main route of every pathway, and apart from it.
"""
import re
from collections import Counter, defaultdict

# step = (substrate, product, compartment, alt)
def S(a, b, comp='c', alt=False):
    return (a, b, comp, alt)


PATHWAYS = [
    ('Glycolysis', [
        S('glc_D', 'g6p'), S('g6p', 'f6p'), S('f6p', 'fdp'), S('fdp', 'g3p'), S('fdp', 'dhap'), S('dhap', 'g3p'),
        S('g3p', '13dpg'), S('13dpg', '3pg'), S('3pg', '2pg'), S('2pg', 'pep'), S('pep', 'pyr'),
        S('g3p', '3pg', alt=True),              # non-phosphorylating GAPDH
        S('fdp', 'f6p', alt=True),              # fructose-bisphosphatase
        S('pyr', 'pep', alt=True),              # PPS / PPDK
    ]),
    ('Pentose phosphate pathway', [
        S('g6p', '6pgl'), S('6pgl', '6pgc'), S('6pgc', 'ru5p_D'), S('ru5p_D', 'xu5p_D'), S('ru5p_D', 'r5p'),
        S('r5p', 's7p'), S('s7p', 'e4p'), S('e4p', 'f6p'), S('xu5p_D', 'g3p'),
    ]),
    ('Pyruvate and fermentation', [
        S('pyr', 'accoa'), S('accoa', 'actp'), S('actp', 'ac'), S('pyr', 'lac_L'), S('pyr', 'lac_D'),
        S('accoa', 'acald'), S('acald', 'etoh'), S('pyr', 'for'), S('pyr', 'acald', alt=True),
        S('pyr', 'alac_S'), S('alac_S', 'actn_R'), S('actn_R', 'btd_RR'),
        S('for', 'co2', alt=True),
    ]),
    ('Anaplerosis and gluconeogenesis', [
        S('pep', 'oaa'), S('oaa', 'pep'), S('pyr', 'oaa'), S('mal_L', 'pyr'), S('oaa', 'pyr', alt=True),
    ]),
    ('TCA cycle', [
        S('oaa', 'cit'), S('cit', 'icit'), S('icit', 'akg'), S('akg', 'succoa'), S('succoa', 'succ'),
        S('succ', 'fum'), S('fum', 'succ'), S('fum', 'mal_L'), S('mal_L', 'oaa'),
        S('icit', 'glx', alt=True), S('glx', 'mal_L', alt=True),          # glyoxylate shunt
        S('asp_L', 'fum', alt=True),                                      # aspartase
        S('mal_L', 'fum', alt=True),
    ]),
    ('Nitrogen assimilation', [
        S('akg', 'glu_L'), S('glu_L', 'gln_L'), S('gln_L', 'glu_L'), S('oaa', 'asp_L'), S('asp_L', 'asn_L'),
        S('pyr', 'ala_L'), S('ala_L', 'pyr', alt=True), S('glu_L', 'akg', alt=True),
    ]),
    ('Galactose (Leloir)', [
        S('lcts', 'gal'), S('gal', 'gal1p'), S('gal1p', 'g1p'), S('gal1p', 'udpgal'), S('udpgal', 'udpg'),
        S('udpg', 'g1p'), S('g1p', 'g6p'),
    ]),
    ('Disaccharides and maltodextrins', [
        S('malt', 'glc_D'), S('sucr', 'glc_D'), S('sucr', 'fru'), S('fru', 'f6p'), S('glc_D', 'fru', alt=True),
        S('cellb', 'glc_D'), S('malttr', 'malt'), S('maltttr', 'malttr'), S('maltpt', 'maltttr'),
        S('malthx', 'maltpt'), S('maltpt', 'g1p'), S('malthx', 'g1p'),
    ]),
    ('Fructans and starch (outside the cell)', [
        S('kesto', 'fru', 'e'), S('kesto', 'glc_D', 'e'), S('kestottr', 'fru', 'e'), S('kestopt', 'fru', 'e'),
        S('inulin', 'fru', 'e'), S('strch2', 'malt', 'e'), S('malttr', 'malt', 'e'), S('malttr', 'glc_D', 'e'),
    ]),
    ('Uronic acids', [
        S('galur', 'tagur'), S('tagur', 'altrn'), S('altrn', '2ddglcn'), S('glcur', 'fruur'), S('fruur', 'mana'),
        S('mana', '2ddglcn'), S('2ddglcn', '2ddg6p'), S('2ddg6p', 'pyr'), S('2ddg6p', 'g3p'),
    ]),
    ('Butyrate', [
        S('accoa', 'aacoa'), S('aacoa', '3hbcoa'), S('3hbcoa', 'b2coa'), S('b2coa', 'btcoa'), S('btcoa', 'but'),
        S('btcoa', 'butp', alt=True), S('butp', 'but', alt=True),
    ]),
    ('Bifid shunt', [
        S('f6p', 'e4p'), S('f6p', 'actp'), S('e4p', 's7p'), S('s7p', 'r5p'), S('r5p', 'ru5p_D'),
        S('ru5p_D', 'xu5p_D'), S('xu5p_D', 'actp'), S('actp', 'ac'),
    ]),
    ('Rhamnose and 1,2-propanediol', [
        S('rmn', 'rml'), S('rml', 'rml1p'), S('rml1p', 'lald_L'), S('rml1p', 'dhap'), S('lald_L', '12ppd_S'),
    ]),
]

# names some GEMs give the same metabolite (normalised: lower case, letters and digits only)
EXTRA_NAMES = {
    'dhap': ['glyceronephosphate', 'dihydroxyacetonephosphate'],
    'g3p': ['glyceraldehyde3phosphate', 'dglyceraldehyde3phosphate'],
    '13dpg': ['13bisphosphodglycerate', '3phosphodglyceroylphosphate', '13diphosphateglycerate'],
    '3pg': ['3phosphodglycerate', '3phosphoglycerate', 'glycerate3phosphate'],
    '2pg': ['2phosphodglycerate', 'dglycerate2phosphate', '2phosphoglycerate'],
    'pep': ['phosphoenolpyruvate'],
    'fdp': ['dfructose16bisphosphate', 'fructose16bisphosphate', 'dfructose16diphosphate'],
    'akg': ['2oxoglutarate', 'alphaketoglutarate', '2ketoglutarate'],
    'oaa': ['oxaloacetate'],
    'actp': ['acetylphosphate'],
    'accoa': ['acetylcoa'],
    'aacoa': ['acetoacetylcoa'],
    'succoa': ['succinylcoa'],
    'btcoa': ['butanoylcoa', 'butyrylcoa'],
    'b2coa': ['crotonoylcoa', 'crotonylcoa', 'but2enoylcoa'],
    '3hbcoa': ['s3hydroxybutanoylcoa', '3hydroxybutyrylcoa'],
    'but': ['butyrate', 'butanoate'],
    'for': ['formate'],
    'acald': ['acetaldehyde'],
    'etoh': ['ethanol'],
    'ac': ['acetate'],
    'lcts': ['lactose'],
    'malt': ['maltose'],
    'glc_D': ['dglucose', 'glucose'],
    'fru': ['dfructose'],
    'gal': ['dgalactose', 'galactose'],
    'g1p': ['dglucose1phosphate', 'glucose1phosphate'],
    'gal1p': ['dgalactose1phosphate', 'alphadgalactose1phosphate'],
    'udpg': ['udpglucose'],
    'udpgal': ['udpgalactose'],
    'cellb': ['cellobiose'],
}


def norm(name):
    return re.sub(r'[^a-z0-9]', '', (name or '').lower())


_NAME_INDEX = None


def name_index(species_fn):
    """Normalised name -> BiGG base, from the BiGG GEMs of the community."""
    global _NAME_INDEX
    if _NAME_INDEX is None:
        votes = defaultdict(Counter)
        for sp in ('Bt', 'St', 'Fp', 'Ec', 'Kp'):
            s = species_fn(sp)
            for k, m in s.mets.items():
                n = norm(m.get('name'))
                if n:
                    votes[n][s.base(k)] += 1
        index = {n: c.most_common(1)[0][0] for n, c in votes.items()}
        for base, names in EXTRA_NAMES.items():
            for n in names:
                index[n] = base
        _NAME_INDEX = index
    return _NAME_INDEX


def canonical(s, species_fn):
    """local metabolite id -> BiGG base, for one species."""
    index = name_index(species_fn)
    wanted = {a for _, steps in PATHWAYS for st in steps for a in st[:2]}
    out = {}
    for k, m in s.mets.items():
        base = s.base(k)
        if base in wanted:
            out[k] = base
            continue
        n = norm(m.get('name'))
        if n in index:
            out[k] = index[n]
    return out


def step_reactions(s, canon, step, min_flux=0.0):
    """GEM reactions of species s that turn the step's substrate into its product, in its compartment."""
    a, b, comp, _ = step
    out = []
    for rid, r in s.rxns.items():
        comps = s.compartments_of(rid)
        if comps != [comp]:
            continue
        subs = {canon.get(k) for k, c in r['metabolites'].items() if c < 0}
        prods = {canon.get(k) for k, c in r['metabolites'].items() if c > 0}
        lb, ub = r['lower_bound'], r['upper_bound']
        forward = a in subs and b in prods and ub > 0
        backward = a in prods and b in subs and lb < 0
        if forward or backward:
            if abs(s.flux.get(rid, 0.0)) >= min_flux:
                out.append(rid)
    return out
