/** Resolve graphical circle positions against compartment outlines and holes without changing biology. */
import { inset, clampToShape, shapeBoundaryPoint, shapeSignedDistance } from './geometry.js';

/** Coordinate tolerance for checking a graphical circle against its cage. */
const EPS = 1e-6;

export function pointFitsRegion(point, region) {
  const inside = clampToShape(point, region.bounds, region.pad, region.boundsCorners);
  if (Math.abs(inside.x - point.x) > EPS || Math.abs(inside.y - point.y) > EPS) {
    return false;
  }
  return region.holes.every((hole, index) => {
    const rect = region.holeInset ? inset(hole, region.holeInset) : hole;
    return shapeSignedDistance(rect, point, region.holeCorners[index]) >= region.clearance + region.holeInset - EPS;
  });
}

/** Pull a point into its outline and out of nested compartments; return null when no attempt fits. */
export function resolveRegion(point, region) {
  let out = clampToShape(point, region.bounds, region.pad, region.boundsCorners);
  for (let pass = 0; pass < 6; pass += 1) {
    let worst = null;
    let worstCorners = null;
    let depth = 0;
    for (let index = 0; index < region.holes.length; index += 1) {
      const hole = region.holes[index];
      const rect = region.holeInset ? inset(hole, region.holeInset) : hole;
      const corners = region.holeCorners[index];
      const distance = region.clearance + region.holeInset - shapeSignedDistance(rect, out, corners);
      if (distance > depth) {
        depth = distance;
        worst = rect;
        worstCorners = corners;
      }
    }
    if (!worst) {
      return out;
    }
    const edge = shapeBoundaryPoint(worst, out, worstCorners);
    const clearance = region.clearance + region.holeInset;
    out = clampToShape(
      { x: edge.x + edge.nx * clearance, y: edge.y + edge.ny * clearance },
      region.bounds,
      region.pad,
      region.boundsCorners,
    );
  }
  return pointFitsRegion(out, region) ? out : null;
}
