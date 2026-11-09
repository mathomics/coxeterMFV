/**
 * First-class Yeast9 / yeast-GEM integration.
 *
 * The source is pinned so the scenario and its representative map remain
 * reproducible. The preview intentionally contains a curated subset of the
 * genome-scale network: enough biology to make every compartment informative
 * without turning the canvas into a 4,000-reaction hairball.
 */

export const YEAST9_SCENARIO_ID = 'yeast9-9-1-1';
export const YEAST9_MODEL_KEY = 'yeast9-9-1-1';

export const YEAST9_SOURCE = Object.freeze({
  version: '9.1.1',
  commit: '2d594ae1c4a2d550ccef120d96a58c7bbf586255',
  filename: 'yeast-GEM-9.1.1.xml',
  url: 'https://raw.githubusercontent.com/SysBioChalmers/yeast-GEM/2d594ae1c4a2d550ccef120d96a58c7bbf586255/model/yeast-GEM.xml',
});

export const YEAST9_PREVIEW_GROUPS = Object.freeze([
  {
    compartment: 'e',
    label: 'Extracellular hydrolysis',
    reactions: ['r_1024', 'r_0370'],
  },
  {
    compartment: 'ce',
    label: 'Cell wall synthesis',
    reactions: ['r_0005', 'r_0006'],
  },
  {
    compartment: 'ce',
    label: 'Plasma-membrane nutrient transport',
    reactions: ['r_1166', 'r_1979', 'r_1244'],
  },
  {
    compartment: 'c',
    label: 'Cytosolic glycolysis and fermentation',
    reactions: ['r_0534', 'r_0467', 'r_0886', 'r_0486', 'r_0962', 'r_0959', 'r_2115'],
  },
  {
    compartment: 'er',
    label: 'ER sphingolipid and sterol metabolism',
    reactions: ['r_0041', 'r_0244', 'r_0263'],
  },
  {
    compartment: 'erm',
    label: 'ER-membrane steryl ester metabolism',
    reactions: ['r_0127', 'r_0132'],
  },
  {
    compartment: 'g',
    label: 'Golgi mannosylation',
    reactions: ['r_0192', 'r_0747'],
  },
  {
    compartment: 'gm',
    label: 'Golgi-membrane phospholipid metabolism',
    reactions: ['r_2360', 'r_2363'],
  },
  {
    compartment: 'lp',
    label: 'Lipid-particle storage metabolism',
    reactions: ['r_2400', 'r_0281'],
  },
  {
    compartment: 'm',
    label: 'Mitochondrial TCA and respiration',
    reactions: ['r_0300', 'r_4262', 'r_0713', 'r_1021'],
  },
  {
    compartment: 'mm',
    label: 'Mitochondrial membrane and oxidative phosphorylation',
    reactions: ['r_0226', 'r_0438', 'r_2464'],
  },
  {
    compartment: 'n',
    label: 'Nuclear inositol and NAD metabolism',
    reactions: ['r_0571', 'r_0769'],
  },
  {
    compartment: 'p',
    label: 'Peroxisomal beta-oxidation',
    reactions: ['r_0120', 'r_0057', 'r_0100'],
  },
  {
    compartment: 'v',
    label: 'Vacuolar carbohydrate turnover',
    reactions: ['r_0193', 'r_0464'],
  },
  {
    compartment: 'vm',
    label: 'Vacuolar membrane transport',
    reactions: ['r_1086', 'r_4237'],
  },
]);

export const YEAST9_PREVIEW_COMPARTMENTS = Object.freeze([
  'e', 'ce', 'c', 'er', 'erm', 'g', 'gm', 'lp', 'm', 'mm', 'n', 'p', 'v', 'vm',
]);

/**
 * The preview reactions the model actually has, one entry per reaction:
 * {reactionId, compartment, group, index, count}, where index and count place
 * it within its group. Empty without a model.
 */
export function yeast9PreviewSpecs(model) {
  if (!model) return [];
  const out = [];
  for (const group of YEAST9_PREVIEW_GROUPS) {
    const present = group.reactions.filter((id) => model.reactions.has(id));
    present.forEach((reactionId, index) => {
      out.push({
        reactionId,
        compartment: group.compartment,
        group: group.label,
        index,
        count: present.length,
      });
    });
  }
  return out;
}
