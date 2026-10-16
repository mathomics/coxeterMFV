//! The interface can be drawn with no window: every interface test and
//! screenshot relies on this.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

#[test]
fn renders_and_clicks_without_a_window() {
    let mut clicks = 0;
    let mut harness = Harness::builder().with_size(egui::vec2(480.0, 200.0)).build_ui_state(
        |ui, clicks: &mut i32| {
            ui.heading("Coxeter-MFV");
            if ui.button("Simulate").clicked() {
                *clicks += 1;
            }
            ui.label(format!("runs: {clicks}"));
            let (rect, _) = ui.allocate_exact_size(egui::vec2(200.0, 60.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 20.0, egui::Color32::from_rgb(2, 132, 199));
        },
        clicks,
    );
    harness.run();
    harness.get_by_label("Simulate").click();
    harness.run();
    assert!(harness.query_by_label("runs: 1").is_some(), "the click reached the interface");
    clicks = *harness.state();
    assert_eq!(clicks, 1);

    let image = harness.render().expect("offscreen rendering through wgpu");
    assert_eq!((image.width(), image.height()), (480, 200));
    if let Ok(dir) = std::env::var("COXETER_SHOT_DIR") {
        image.save(std::path::Path::new(&dir).join("offscreen.png")).unwrap();
    }
}
