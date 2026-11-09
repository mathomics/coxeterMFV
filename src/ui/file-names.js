/**
 * Names for exported files: what they are of, then a short suffix for what they
 * are (`yeastGEM_v9.1.1_map.json`, `yeastGEM_v9.1.1_model.json`,
 * `consorcio-anaerobio_scenario.json`), so the files saved from one model sort
 * together and say at a glance which is the map and which the model.
 */

export const EXPORT_SUFFIX = {
  map: 'map',            // Map JSON, SVG and PNG
  model: 'model',        // model JSON and SBML
  scenario: 'scenario',
  fluxes: 'fluxes',
  bounds: 'bounds',      // reaction bounds (CSV)
  repro: 'repro',        // reproducibility bundle
  methods: 'methods',    // methods / provenance report
  findings: 'findings',
};

/**
 * A model or community name made safe for a file name: accents dropped, any
 * run of characters other than letters, digits, `.`, `-`, `_` and `+` turned
 * into a single `_`, and kept to a readable length. Case is kept (`iML1515`),
 * and so is the `+` that joins several uploaded models.
 */
export function fileStem(text, fallback = 'coxeter') {
  const stem = String(text ?? '')
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .replace(/[^A-Za-z0-9._+-]+/g, '_')
    .replace(/_{2,}/g, '_')
    .replace(/^[._+-]+|[._+-]+$/g, '')
    .slice(0, 80)
    .replace(/[._+-]+$/g, '');
  return stem || fallback;
}

/** `<name>_<suffix>.<extension>`. */
export function exportFileName(name, suffix, extension) {
  return `${fileStem(name)}_${fileStem(suffix, 'export')}.${extension}`;
}

/**
 * For exports kept as a record of a moment (the reproducibility bundle, the
 * methods report): `<name>_<suffix>_<yyyymmdd-hhmm>.<extension>`, local time,
 * so a bundle taken before a change and one taken after do not share a name.
 */
export function timestampedFileName(name, suffix, extension, date = new Date()) {
  const pad = (value) => String(value).padStart(2, '0');
  const stamp = `${date.getFullYear()}${pad(date.getMonth() + 1)}${pad(date.getDate())}`
    + `-${pad(date.getHours())}${pad(date.getMinutes())}`;
  return exportFileName(name, `${suffix}_${stamp}`, extension);
}

/**
 * A member model of a published community: `<community>_<member>_model.<ext>`.
 * Member ids are short codes (Bt, Ec) that recur across communities, so the
 * community stays in the name.
 */
export function memberModelFileName(communityId, memberId, extension) {
  return exportFileName(`${fileStem(communityId)}_${fileStem(memberId, 'member')}`, EXPORT_SUFFIX.model, extension);
}

/** A data file of a published community: `media.csv` → `<community>_media.csv`. */
export function communityDataFileName(communityId, file) {
  const match = /^(.*?)(?:\.([A-Za-z0-9]+))?$/.exec(String(file ?? ''));
  return exportFileName(communityId, match?.[1] || 'data', match?.[2] || 'csv');
}
