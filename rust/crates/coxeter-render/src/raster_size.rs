//! How large a PNG of the map can be made (`src/render/raster-size.js`).
//!
//! The PNG is the SVG drawn into a canvas, and a canvas has a ceiling: about
//! 268 million pixels in Chromium, about 124 million in Firefox, 16.7 million in
//! Safari, and 32767 pixels a side. A hand-built genome-scale map is easily
//! 10000 × 10000, which at the 2× the export asks for is 400 million pixels:
//! the canvas would come back unusable and the export fail on the very maps
//! that most need a picture. Each entry here is the largest scale, up to the
//! 2× that is wanted, that fits one ceiling. The export tries them in order
//! and keeps the first one that can be made. The native export keeps the same
//! ladder, so a PNG made here is the size the web app would have made.
//!
//! The first ceiling is memory rather than a browser's limit. A picture at
//! Chromium's ceiling holds over a gigabyte of pixels before it is encoded,
//! and Firefox, instead of refusing one, can take the tab down with it. A
//! hundred million pixels is still several times what a printed figure needs,
//! and the SVG keeps full resolution.

use coxeter_core::js;

/// The scale the export asks for, device pixels per map unit.
pub const PREFERRED_SCALE: f64 = 2.0;

struct Ceiling {
    area: f64,
    side: f64,
}

const CEILINGS: [Ceiling; 2] = [
    Ceiling { area: 100_000_000.0, side: 32_767.0 }, // what fits in memory, under Chromium's and Firefox's ceilings
    Ceiling { area: 16_777_216.0, side: 16_384.0 },  // Safari, iOS
];

/// Scales to try, largest first, each at most `preferred`
/// (`rasterScales(width, height, preferred = 2)`).
pub fn raster_scales(width: f64, height: f64, preferred: f64) -> Vec<f64> {
    let positive = |v: f64| if v == 0.0 || v.is_nan() { 1.0 } else { v };
    let w = js::max(1.0, positive(width));
    let h = js::max(1.0, positive(height));
    let mut scales: Vec<f64> = Vec::new();
    for c in &CEILINGS {
        let fit = js::min_n([preferred, c.side / w, c.side / h, (c.area / (w * h)).sqrt()]);
        // Rounded down, so the rounded pixel size never lands just over the ceiling.
        let scale = (fit * 100.0).floor() / 100.0;
        if scale > 0.0 && !scales.contains(&scale) {
            scales.push(scale);
        }
    }
    scales
}

#[cfg(test)]
mod tests {
    use super::*;

    // From test/ui-audit-fixes.test.mjs ("PNG scales fit each browser canvas ceiling").
    #[test]
    fn png_scales_fit_each_browser_ceiling_largest_first() {
        assert_eq!(PREFERRED_SCALE, 2.0);
        assert_eq!(raster_scales(1200.0, 800.0, PREFERRED_SCALE), [2.0], "an ordinary map gets the full 2x");
        let scales = raster_scales(10000.0, 10200.0, PREFERRED_SCALE);
        assert_eq!(scales, [0.99, 0.4]);
        for (scale, area, side) in [(0.99, 100_000_000.0, 32_767.0), (0.4, 16_777_216.0, 16_384.0)] {
            let w = js::round(10000.0 * scale);
            let h = js::round(10200.0 * scale);
            assert!(w * h <= area && w <= side && h <= side, "{scale}x must fit {area} px");
        }
        assert_eq!(raster_scales(40000.0, 200.0, PREFERRED_SCALE), [0.81, 0.4], "a very wide map is held by the side limit");
        assert!(raster_scales(1e7, 1e7, PREFERRED_SCALE).is_empty(), "nothing fits: the export says so");
    }
}
