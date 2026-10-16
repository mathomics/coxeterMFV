//! The painter against resvg: the E. coli map of the browser references,
//! drawn by [`MapView`] through egui on wgpu (offscreen, `egui_kittest`), and
//! the same element tree — styled by the same resolver of `assets/map.css`,
//! inlined, at the same zoom — rasterised by resvg, the reference rasteriser
//! the app's PNG export uses.
//!
//! Set `COXETER_SHOT_DIR` to keep the two pictures and their difference.
//!
//! The two cannot be identical: egui anti-aliases by feathering mesh edges
//! (about one pixel), resvg by coverage; text goes through different shapers
//! and outline renderers (harfrust + lyon here, rustybuzz + tiny-skia there);
//! the soft shadows and glows are layered approximations here and Gaussian
//! blurs there. The test bounds the differences those cause (see the numbers
//! asserted) so that anything structural — a missing layer, a wrong colour, a
//! shape out of place, a label at the wrong size — fails it.

mod common;

use coxeter_canvas::camera::{Camera, ZoomVars, min_hit_radius};
use coxeter_canvas::host::{MapHost, RenderOptions};
use coxeter_canvas::style::PageTheme;
use coxeter_canvas::test_host::EditableMapHost;
use coxeter_canvas::MapView;
use coxeter_render::css::map_stylesheet;
use coxeter_render::{InlineOptions, inline_styles, render_map};
use egui::Vec2;
use egui_kittest::Harness;
use image::{Rgba, RgbaImage};
use serde_json::json;

use common::*;

struct App {
    view: MapView,
    host: EditableMapHost,
}

/// The stage of the harness: its 8-pixel margin to the window's edge.
const MARGIN: u32 = 8;

fn reference(host: &EditableMapHost, cam: Camera, theme: &PageTheme, w: u32, h: u32, bg: [u8; 3]) -> RgbaImage {
    use resvg::{tiny_skia, usvg};
    let map = host.map().unwrap();
    let state = host.render_state(RenderOptions { reaction_edit: false, min_hit_radius: min_hit_radius(cam.scale) });
    let tree = render_map(map.graph(), &state);
    let inlined = inline_styles(
        &tree,
        map_stylesheet(),
        &InlineOptions { resolve: theme.options(ZoomVars::at(cam.scale), None), device_scale: cam.scale },
    );
    let mut opt = usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    let parsed = usvg::Tree::from_str(&inlined.to_xml(), &opt).expect("the tree parses");
    let mut pixmap = tiny_skia::Pixmap::new(w, h).unwrap();
    pixmap.fill(tiny_skia::Color::from_rgba8(bg[0], bg[1], bg[2], 255));
    let t = tiny_skia::Transform::from_row(cam.scale as f32, 0.0, 0.0, cam.scale as f32, cam.x as f32, cam.y as f32);
    resvg::render(&parsed, t, &mut pixmap.as_mut());
    let mut img = RgbaImage::new(w, h);
    for (k, p) in pixmap.pixels().iter().enumerate() {
        let c = p.demultiply();
        img.put_pixel(k as u32 % w, k as u32 / w, Rgba([c.red(), c.green(), c.blue(), 255]));
    }
    img
}

/// Mean absolute difference (0–255, over RGB) and the share of pixels that
/// differ visibly (some channel by more than 48).
fn compare(a: &RgbaImage, b: &RgbaImage, region: (u32, u32, u32, u32)) -> (f64, f64, RgbaImage) {
    let (x0, y0, x1, y1) = region;
    let mut diff = RgbaImage::new(a.width(), a.height());
    let (mut sum, mut visible, mut n) = (0.0, 0usize, 0usize);
    for y in y0..y1 {
        for x in x0..x1 {
            let (p, q) = (a.get_pixel(x, y), b.get_pixel(x, y));
            let d: Vec<i32> = (0..3).map(|c| (p[c] as i32 - q[c] as i32).abs()).collect();
            let m = *d.iter().max().unwrap();
            sum += d.iter().sum::<i32>() as f64 / 3.0;
            n += 1;
            if m > 48 {
                visible += 1;
            }
            let v = (255 - m.min(255)) as u8;
            diff.put_pixel(x, y, Rgba([255, v, v, 255]));
        }
    }
    (sum / n as f64, visible as f64 / n as f64, diff)
}

fn render_case(name: &str, cam: Camera, dark: bool) -> Option<(f64, f64)> {
    render_case_with(name, cam, dark, false)
}

/// `crisp`: wait for the settled layer (the view rasterised by resvg) and
/// shoot that; otherwise the meshes alone.
fn render_case_with(name: &str, cam: Camera, dark: bool, crisp: bool) -> Option<(f64, f64)> {
    let doc = gestures()?;
    let mut host = host_for(&doc, &json!({ "start": "initial" }));
    host.view.dark = dark;
    let mut view = MapView::new();
    view.set_camera(cam);
    view.crisp = crisp;
    let size = Vec2::new(1160.0, 880.0);
    let mut harness = Harness::builder().with_size(size).with_pixels_per_point(1.0).wgpu().build_ui_state(
        |ui, app: &mut App| {
            app.view.show(ui, &mut app.host, 1);
        },
        App { view, host },
    );
    harness.run_steps(4);
    for _ in 0..500 {
        if !crisp || harness.state().view.crisp_shown() {
            break;
        }
        harness.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(harness.state().view.crisp_shown(), crisp);
    let shot = match harness.render() {
        Ok(img) => img,
        Err(e) => {
            eprintln!("skipped {name}: no GPU rendering here ({e})");
            return None;
        }
    };
    let app = harness.state();
    let stage = app.view.canvas.rect;
    assert_eq!((stage.left, stage.top), (MARGIN as f64, MARGIN as f64));
    let (w, h) = (stage.width as u32, stage.height as u32);
    let theme = PageTheme::for_dark(dark);
    let bg = if dark { [0x0b, 0x12, 0x20] } else { [0xf3, 0xf6, 0xfa] };
    let reference = reference(&app.host, app.view.camera(), &theme, w, h, bg);
    // The stage of the shot, as an image of its own.
    let ours = image::imageops::crop_imm(&shot, MARGIN, MARGIN, w, h).to_image();
    let map = app.host.map().unwrap();
    let c = app.view.camera();
    let x1 = ((c.x + map.width() * c.scale).min(w as f64)).max(0.0) as u32;
    let y1 = ((c.y + map.height() * c.scale).min(h as f64)).max(0.0) as u32;
    let region = (c.x.max(0.0) as u32, c.y.max(0.0) as u32, x1, y1);
    let (mean, visible, diff) = compare(&ours, &reference, region);
    eprintln!("{name}: mean |Δ| {mean:.2} of 255, {:.2}% of pixels visibly different", 100.0 * visible);
    if let Ok(dir) = std::env::var("COXETER_SHOT_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        ours.save(dir.join(format!("{name}-egui.png"))).unwrap();
        reference.save(dir.join(format!("{name}-resvg.png"))).unwrap();
        diff.save(dir.join(format!("{name}-diff.png"))).unwrap();
    }
    Some((mean, visible))
}

#[test]
fn the_painter_draws_what_resvg_draws() {
    // Close: labels at full size, curves, arrowheads, membranes.
    let Some((mean, visible)) = render_case("close", Camera { x: -500.0, y: -380.0, scale: 0.75, fitted: true }, false) else { return };
    assert!(mean < 4.0 && visible < 0.04, "close: mean {mean:.2}, visible {:.2}%", 100.0 * visible);
    // The whole cell: the far view, labels scaled up by --label-scale and
    // the small type hidden by `is-far`.
    let Some((mean, visible)) = render_case("far", Camera { x: 0.0, y: 0.0, scale: 0.29, fitted: true }, false) else { return };
    assert!(mean < 4.0 && visible < 0.04, "far: mean {mean:.2}, visible {:.2}%", 100.0 * visible);
    // Dark.
    let Some((mean, visible)) = render_case("dark", Camera { x: -500.0, y: -380.0, scale: 0.75, fitted: true }, true) else { return };
    assert!(mean < 4.0 && visible < 0.04, "dark: mean {mean:.2}, visible {:.2}%", 100.0 * visible);
}

#[test]
fn once_still_the_map_is_what_resvg_draws() {
    // The settled layer is resvg's picture, in place to the pixel: next to
    // nothing differs from the reference.
    let Some((mean, visible)) = render_case_with("crisp", Camera { x: -500.0, y: -380.0, scale: 0.75, fitted: true }, false, true) else { return };
    assert!(mean < 0.5 && visible < 0.002, "crisp: mean {mean:.2}, visible {:.2}%", 100.0 * visible);
}
