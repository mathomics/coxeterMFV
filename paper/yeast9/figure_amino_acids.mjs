/**
 * Add reactions to the Figure 2 drawing that the older manuscript map leaves
 * out: the active amino acid alternatives and the cytosolic NADP-dependent
 * isocitrate dehydrogenase. Only the drawing changes; the model is read, never
 * edited.
 */

import { closestBoundaryPoint } from '../../src/layout/geometry.js';
import { satellitePlace, shapeBranch } from '../../src/layout/reaction-edit.js';

/**
 * Drawing helpers bound to one figure map. `node` places a primary metabolite
 * or returns the one already drawn, `reaction` draws a model reaction between
 * given primary nodes with its other metabolites as satellites, and `added`
 * lists the reactions drawn.
 */
function figureDrawing(map, model, speciesId) {
  const added = [];

  function node(localId, x, y, suffix = '') {
    const id = `met:${speciesId}@${localId}${suffix}`;
    if (!map.nodes.has(id)) {
      if (!Number.isFinite(x) || !Number.isFinite(y)) {
        throw new Error(`The figure needs a position for metabolite ${localId}.`);
      }
      const metabolite = model.metabolites.get(localId);
      map.nodes.set(id, {
        id,
        kind: 'metabolite',
        mergedId: `${speciesId}@${localId}`,
        localId,
        name: metabolite.name,
        base: metabolite.base,
        compartment: metabolite.compartment,
        cellId: speciesId,
        x,
        y,
        r: 12,
        primary: true,
        visualRole: 'primary',
      });
    }
    return id;
  }

  function reaction(localId, x, y, direction, primary, transport = false) {
    const source = model.reactions.get(localId);
    const id = `${speciesId}@${localId}`;
    const a1 = { x: x - direction.x * 17, y: y - direction.y * 17 };
    const a2 = { x: x + direction.x * 17, y: y + direction.y * 17 };
    const drawing = {
      id,
      localId,
      name: source.name,
      cellId: speciesId,
      kind: 'internal',
      compartments: source.compartments,
      x,
      y,
      bounds: { lower: source.lowerBound, upper: source.upperBound },
      axis: { a1, a2, dir: direction },
      axisDirectionHint: direction,
      onMembrane: transport,
    };
    if (transport) {
      drawing.boundaryId = `${speciesId}/m`;
      drawing.membrane = 'm';
      drawing.transportMode = 'uniport';
    }
    map.reactions.set(id, drawing);
    map.segments.push({ reactionId: id, kind: 'axis', from: a1, to: a2, arrow: false });
    const satellites = { substrate: 0, product: 0 };
    for (const [mid, coefficient] of Object.entries(source.metabolites)) {
      const substrate = coefficient < 0;
      let side = 'product';
      let sign = 1;
      if (substrate) {
        side = 'substrate';
        sign = -1;
      }
      let nodeId = primary[mid];
      let kind = 'primary';
      if (!nodeId) {
        const point = satellitePlace(drawing, sign, satellites[side]);
        satellites[side] += 1;
        nodeId = node(mid, point.x, point.y, `:figure:${localId}`);
        kind = 'currency';
        const satellite = map.nodes.get(nodeId);
        satellite.kind = 'currency';
        satellite.primary = false;
        satellite.visualRole = 'secondary';
        satellite.r = 5;
        satellite.reactionId = id;
      }
      const metabolite = map.nodes.get(nodeId);
      if (metabolite.localId !== mid) {
        throw new Error(`The drawing of ${localId} connects the wrong metabolite for ${mid}.`);
      }
      const point = { x: metabolite.x, y: metabolite.y };
      let from = a2;
      let to = point;
      if (substrate) {
        from = point;
        to = a1;
      }
      const segment = {
        reactionId: id,
        kind,
        nodeId,
        coefficient,
        side,
        from,
        to,
        arrow: !substrate,
        backboneAnchor: true,
      };
      shapeBranch(segment, drawing);
      map.segments.push(segment);
    }
    added.push(localId);
  }

  return { node, reaction, added };
}

export function addAminoAcidRoutes(map, model, speciesId) {
  const { node, reaction, added } = figureDrawing(map, model, speciesId);
  const glutamate = node('s_0991');
  const glutamine = node('s_0999');
  const oxoglutarate = node('s_0180');
  reaction('r_0471', 6860, 2150, { x: 0, y: -1 }, { s_0180: oxoglutarate, s_0991: glutamate });
  reaction('r_0476', 6870, 1580, { x: 0, y: -1 }, { s_0991: glutamate, s_0999: glutamine });

  const glutamylPhosphate = node('s_0986', 7110, 1970, ':figure:proline');
  const glutamateSemialdehyde = node('s_0997', 7110, 2190, ':figure:proline');
  const pyrroline = node('s_0118', 7110, 2410, ':figure:proline');
  const proline = node('s_1035', 7110, 2630, ':figure:proline');
  reaction('r_0468', 7110, 1860, { x: 0, y: 1 }, { s_0991: glutamate, s_0986: glutamylPhosphate });
  reaction('r_0473', 7110, 2080, { x: 0, y: 1 }, { s_0986: glutamylPhosphate, s_0997: glutamateSemialdehyde });
  reaction('r_1887', 7110, 2300, { x: 0, y: 1 }, { s_0997: glutamateSemialdehyde, s_0118: pyrroline });
  reaction('r_0957', 7110, 2520, { x: 0, y: 1 }, { s_0118: pyrroline, s_1035: proline });

  const isopropylOxosuccinate = `met:${speciesId}@s_0010:research:leucine`;
  const ketoLeucine = node('s_0291', 6720, 4595, ':figure:leucine');
  const leucine = node('s_1021', 7000, 4595, ':figure:leucine');
  reaction('r_0029', 6570, 4595, { x: 1, y: 0 }, { s_0010: isopropylOxosuccinate, s_0291: ketoLeucine });
  reaction('r_0699', 6860, 4595, { x: -1, y: 0 }, { s_0291: ketoLeucine, s_1021: leucine });

  const ketoIsoleucineM = `met:${speciesId}@s_0060:research:isoleucine`;
  const ketoIsoleucineC = node('s_0056', 6550, 3720, ':figure:isoleucine');
  const isoleucine = node('s_1016', 6870, 3720, ':figure:isoleucine');
  const boundary = map.boundary(`${speciesId}/m`);
  const crossing = closestBoundaryPoint(map, boundary, { x: 6200, y: 3720 });
  reaction('r_1596', crossing.x, crossing.y, { x: -crossing.nx, y: -crossing.ny }, {
    s_0056: ketoIsoleucineC,
    s_0060: ketoIsoleucineM,
  }, true);
  reaction('r_0663', 6710, 3720, { x: -1, y: 0 }, { s_0056: ketoIsoleucineC, s_1016: isoleucine });
  return added;
}

/**
 * Cytosolic NADP-dependent isocitrate dehydrogenase (r_0659), which oxidizes
 * isocitrate in the cytoplasm in the FBA and CycleFreeFlux solutions. It is
 * drawn into the free space beside the cytosolic 2-oxoglutarate it produces.
 */
export function addCytosolicIsocitrateDehydrogenase(map, model, speciesId) {
  const { node, reaction, added } = figureDrawing(map, model, speciesId);
  const oxoglutarate = node('s_0180');
  const target = map.nodes.get(oxoglutarate);
  const isocitrate = node('s_0940', target.x - 240, target.y - 380, ':figure:isocitrate');
  const source = map.nodes.get(isocitrate);
  const dx = target.x - source.x;
  const dy = target.y - source.y;
  const length = Math.hypot(dx, dy);
  reaction('r_0659', (source.x + target.x) / 2, (source.y + target.y) / 2, { x: dx / length, y: dy / length }, {
    s_0940: isocitrate,
    s_0180: oxoglutarate,
  });
  return added;
}
