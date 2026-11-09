/**
 * Shared metabolite table: formula and charge at pH 7.3, BiGG conventions.
 * The model builder looks up every base id here, so a typo in a reaction string
 * fails loudly instead of producing a silently unbalanced model.
 */
export const MET = {
  // Central carbon
  glc__D:   ['C6H12O6', 0,   'D-Glucose'],
  g6p:      ['C6H11O9P', -2, 'D-Glucose 6-phosphate'],
  f6p:      ['C6H11O9P', -2, 'D-Fructose 6-phosphate'],
  fdp:      ['C6H10O12P2', -4, 'D-Fructose 1,6-bisphosphate'],
  dhap:     ['C3H5O6P', -2,  'Dihydroxyacetone phosphate'],
  g3p:      ['C3H5O6P', -2,  'Glyceraldehyde 3-phosphate'],
  '13dpg':  ['C3H4O10P2', -4, '1,3-Bisphospho-D-glycerate'],
  '3pg':    ['C3H4O7P', -3,  '3-Phospho-D-glycerate'],
  '2pg':    ['C3H4O7P', -3,  'D-Glycerate 2-phosphate'],
  pep:      ['C3H2O6P', -3,  'Phosphoenolpyruvate'],
  pyr:      ['C3H3O3', -1,   'Pyruvate'],
  '6pgl':   ['C6H9O9P', -2,  '6-phospho-D-glucono-1,5-lactone'],
  '6pgc':   ['C6H10O10P', -3, '6-Phospho-D-gluconate'],
  ru5p__D:  ['C5H9O8P', -2,  'D-Ribulose 5-phosphate'],
  r5p:      ['C5H9O8P', -2,  'Alpha-D-Ribose 5-phosphate'],
  xu5p__D:  ['C5H9O8P', -2,  'D-Xylulose 5-phosphate'],
  s7p:      ['C7H13O10P', -2, 'Sedoheptulose 7-phosphate'],
  e4p:      ['C4H7O7P', -2,  'D-Erythrose 4-phosphate'],
  rb15bp:   ['C5H8O11P2', -4, 'D-Ribulose 1,5-bisphosphate'],

  // TCA
  accoa:    ['C23H34N7O17P3S', -4, 'Acetyl-CoA'],
  coa:      ['C21H32N7O16P3S', -4, 'Coenzyme A'],
  cit:      ['C6H5O7', -3,   'Citrate'],
  icit:     ['C6H5O7', -3,   'Isocitrate'],
  akg:      ['C5H4O5', -2,   '2-Oxoglutarate'],
  succoa:   ['C25H35N7O19P3S', -5, 'Succinyl-CoA'],
  succ:     ['C4H4O4', -2,   'Succinate'],
  fum:      ['C4H2O4', -2,   'Fumarate'],
  mal__L:   ['C4H4O5', -2,   'L-Malate'],
  oaa:      ['C4H2O5', -2,   'Oxaloacetate'],
  glx:      ['C2HO3', -1,    'Glyoxylate'],

  // Fermentation
  ac:       ['C2H3O2', -1,   'Acetate'],
  actp:     ['C2H3O5P', -2,  'Acetyl phosphate'],
  acald:    ['C2H4O', 0,     'Acetaldehyde'],
  etoh:     ['C2H6O', 0,     'Ethanol'],
  lac__D:   ['C3H5O3', -1,   'D-Lactate'],
  lac__L:   ['C3H5O3', -1,   'L-Lactate'],
  for:      ['CHO2', -1,     'Formate'],
  glyc:     ['C3H8O3', 0,    'Glycerol'],
  glyc3p:   ['C3H7O6P', -2,  'Glycerol 3-phosphate'],

  // Nucleotides and redox
  atp:      ['C10H12N5O13P3', -4, 'ATP'],
  adp:      ['C10H12N5O10P2', -3, 'ADP'],
  amp:      ['C10H12N5O7P', -2, 'AMP'],
  gtp:      ['C10H12N5O14P3', -4, 'GTP'],
  gdp:      ['C10H12N5O11P2', -3, 'GDP'],
  nad:      ['C21H26N7O14P2', -1, 'NAD+'],
  nadh:     ['C21H27N7O14P2', -2, 'NADH'],
  nadp:     ['C21H25N7O17P3', -3, 'NADP+'],
  nadph:    ['C21H26N7O17P3', -4, 'NADPH'],
  fad:      ['C27H31N9O15P2', -2, 'FAD'],
  fadh2:    ['C27H33N9O15P2', -2, 'FADH2'],
  q8:       ['C49H74O4', 0,  'Ubiquinone-8'],
  q8h2:     ['C49H76O4', 0,  'Ubiquinol-8'],
  q6:       ['C39H58O4', 0,  'Ubiquinone-6'],
  q6h2:     ['C39H60O4', 0,  'Ubiquinol-6'],
  pq:       ['C53H80O2', 0,  'Plastoquinone'],
  pqh2:     ['C53H82O2', 0,  'Plastoquinol'],
  // Carrier proteins have no meaningful elemental formula; an empty formula
  // tells the balance checker to skip the reactions that use them.
  pc_ox:    ['', 1,          'Plastocyanin (oxidised)'],
  pc_rd:    ['', 0,          'Plastocyanin (reduced)'],
  fdxo:     ['', -2,         'Ferredoxin (oxidised)'],
  fdxr:     ['', -3,         'Ferredoxin (reduced)'],
  f420o:    ['', 0,          'Coenzyme F420 (oxidised)'],
  f420r:    ['', 0,          'Coenzyme F420 (reduced)'],

  // Small molecules and ions
  h:        ['H', 1,         'H+'],
  h2o:      ['H2O', 0,       'H2O'],
  co2:      ['CO2', 0,       'CO2'],
  hco3:     ['CHO3', -1,     'Bicarbonate'],
  o2:       ['O2', 0,        'O2'],
  h2:       ['H2', 0,        'H2'],
  ch4:      ['CH4', 0,       'Methane'],
  n2:       ['N2', 0,        'N2'],
  nh4:      ['H4N', 1,       'Ammonium'],
  no3:      ['NO3', -1,      'Nitrate'],
  pi:       ['HO4P', -2,     'Phosphate'],
  ppi:      ['HO7P2', -3,    'Diphosphate'],
  so4:      ['O4S', -2,      'Sulfate'],
  h2s:      ['H2S', 0,       'Hydrogen sulfide'],
  k:        ['K', 1,         'Potassium'],
  na1:      ['Na', 1,        'Sodium'],
  photon:   ['', 0,          'Photon'],

  // Amino acids and nitrogen
  glu__L:   ['C5H8NO4', -1,  'L-Glutamate'],
  gln__L:   ['C5H10N2O3', 0, 'L-Glutamine'],
  asp__L:   ['C4H6NO4', -1,  'L-Aspartate'],
  ala__L:   ['C3H7NO2', 0,   'L-Alanine'],
  ser__L:   ['C3H7NO3', 0,   'L-Serine'],

  // Storage and polymers
  starch:   ['C6H10O5', 0,   'Starch (glucose unit)'],
  glycogen: ['C6H10O5', 0,   'Glycogen (glucose unit)'],
  g1p:      ['C6H11O9P', -2, 'D-Glucose 1-phosphate'],
  udpg:     ['C15H22N2O17P2', -2, 'UDP-glucose'],
  utp:      ['C9H11N2O15P3', -4, 'UTP'],
  udp:      ['C9H11N2O12P2', -3, 'UDP'],
  '3pgal':  ['C3H5O4', -1,   'Glycerate'],
};

/** Metabolites that should always be drawn as small per-reaction satellites. */
export const CURRENCY = new Set([
  'h', 'h2o', 'atp', 'adp', 'amp', 'gtp', 'gdp', 'pi', 'ppi', 'nad', 'nadh',
  'nadp', 'nadph', 'co2', 'o2', 'coa', 'nh4', 'q8', 'q8h2', 'q6', 'q6h2',
  'pq', 'pqh2', 'fad', 'fadh2', 'fdxo', 'fdxr', 'f420o', 'f420r', 'so4',
  'photon', 'k', 'na1', 'utp', 'udp', 'pc_ox', 'pc_rd',
]);
