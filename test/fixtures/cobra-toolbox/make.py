"""Write mini.mat and mini.xlsx: one five-reaction model in the COBRA Toolbox
layouts (a saved model struct, and writeCbModel's Excel sheets).

    python test/fixtures/cobra-toolbox/make.py
"""
from pathlib import Path

import numpy as np
from openpyxl import Workbook
from scipy.io import savemat
from scipy.sparse import csc_matrix

HERE = Path(__file__).parent

mets = ['glc__D[e]', 'glc__D[c]', 'g6p[c]', 'atp[c]', 'adp[c]']
met_names = ['D-Glucose', 'D-Glucose', 'D-Glucose 6-phosphate', 'ATP', 'ADP']
formulas = ['C6H12O6', 'C6H12O6', 'C6H11O9P', 'C10H12N5O13P3', 'C10H12N5O10P2']
charges = [0, 0, -2, -4, -3]
kegg = ['C00031', 'C00031', 'C00092', 'C00002', 'C00008']
rxns = ['EX_glc__D_e', 'GLCt', 'HEX1', 'ATPS', 'BIOMASS']
rxn_names = ['D-Glucose exchange', 'Glucose transport', 'Hexokinase', 'ATP synthase (lumped)', 'Biomass']
stoich = [
    {'glc__D[e]': -1},
    {'glc__D[e]': -1, 'glc__D[c]': 1},
    {'glc__D[c]': -1, 'atp[c]': -1, 'g6p[c]': 1, 'adp[c]': 1},
    {'adp[c]': -2, 'atp[c]': 2},
    {'g6p[c]': -1},
]
formulas_rxn = ['glc__D[e] <=> ', 'glc__D[e] -> glc__D[c]', 'atp[c] + glc__D[c] -> adp[c] + g6p[c]',
                '2 adp[c] -> 2 atp[c]', 'g6p[c] -> ']
lb = [-10, 0, 0, 0, 0]
ub = [1000] * 5
c = [0, 0, 0, 0, 1]
genes = ['g1', 'g2', 'g3', 'g4', 'g5', 'g6']
gr_rules = ['', '(g1 and g2) or g3', 'g4', 'g5 or g6', '']
rules = ['', '(x(1) & x(2)) | x(3)', 'x(4)', 'x(5) | x(6)', '']
subsystems = ['Exchange', 'Transport', 'Glycolysis', 'Energy', 'Biomass']
ec = ['', '', '2.7.1.1', '', '']


def cell(values):
    out = np.empty((len(values), 1), dtype=object)
    for i, value in enumerate(values):
        out[i, 0] = value
    return out


def column(values):
    return np.array(values, dtype=float).reshape(-1, 1)


S = np.zeros((len(mets), len(rxns)))
for j, entries in enumerate(stoich):
    for met, coef in entries.items():
        S[mets.index(met), j] = coef

model = {
    'modelID': 'mini', 'modelName': 'Mini COBRA Toolbox model', 'description': 'mini.mat',
    'rxns': cell(rxns), 'mets': cell(mets), 'S': csc_matrix(S),
    'lb': column(lb), 'ub': column(ub), 'c': column(c), 'b': column([0] * len(mets)),
    'csense': 'E' * len(mets), 'osenseStr': 'max', 'rev': column([1, 0, 0, 0, 0]),
    'rxnNames': cell(rxn_names), 'metNames': cell(met_names), 'metFormulas': cell(formulas),
    'metCharges': column(charges), 'metKEGGID': cell(kegg),
    'genes': cell(genes), 'grRules': cell(gr_rules), 'rules': cell(rules),
    # COBRA Toolbox 3 keeps each reaction's subsystems as a cell of its own.
    'subSystems': cell([cell([s]) for s in subsystems]), 'rxnECNumbers': cell(ec),
    'comps': cell(['c', 'e']), 'compNames': cell(['Cytosol', 'Extracellular']),
}
savemat(HERE / 'mini.mat', {'model': model}, do_compression=True, oned_as='column')

book = Workbook()
sheet = book.active
sheet.title = 'Reaction List'
sheet.append(['Abbreviation', 'Description', 'Reaction', 'GPR', 'Genes', 'Protein', 'Subsystem', 'Reversible',
              'Lower bound', 'Upper bound', 'Objective', 'Confidence Score', 'EC Number', 'Notes', 'References'])
for j, rid in enumerate(rxns):
    sheet.append([rid, rxn_names[j], formulas_rxn[j], gr_rules[j], None, None, subsystems[j], 1 if lb[j] < 0 else 0,
                  lb[j], ub[j], c[j], None, ec[j] or None, None, None])
mets_sheet = book.create_sheet('Metabolite List')
mets_sheet.append(['Abbreviation', 'Description', 'Neutral formula', 'Charged formula', 'Charge', 'Compartment',
                   'KEGG ID', 'PubChem ID', 'ChEBI ID', 'InChI string', 'SMILES', 'HMDB ID'])
for i, mid in enumerate(mets):
    mets_sheet.append([mid, met_names[i], None, formulas[i], charges[i],
                       'Extracellular' if mid.endswith('[e]') else 'Cytosol', kegg[i], None, None, None, None, None])
book.save(HERE / 'mini.xlsx')
