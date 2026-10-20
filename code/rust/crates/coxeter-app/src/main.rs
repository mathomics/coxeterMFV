//! Coxeter Multicell Flux Visualizer.
//!
//! ```text
//! coxeter [options] [files…]
//!   files…                 models (COBRA JSON, SBML), a map, a scenario, a
//!                          reproducibility bundle or omics tables, opened in turn
//!   --screenshot OUT.png   draw one frame offscreen, write it, and exit
//!   --size WxH             the window (or screenshot) size, default 1440x900
//!   --scenario ID          the scenario to open (default: the first)
//!   --dark / --light       the theme
//!   --models DIR           the folder holding models/ (default: built in, or the repository)
//!   --solver NAME          solve on highs, gurobi or cplex (default: the one chosen last)
//!   --solvers              list the solvers found on this computer and exit
//!   --check-solvers        check each solver found on a battery of problems and exit
//!   --version              print the version and exit
//! ```

use std::path::PathBuf;
use std::time::Duration;

use coxeter_app::files::{FileDialogs, NativeDialogs, ScriptedDialogs};
use coxeter_app::solvers::Solvers;
use coxeter_app::{Shell, assets};
use coxeter_solver::backends::SolverKind;
use coxeter_solver::selftest::self_test;
use coxeter_state::{CachedFetcher, Store};

struct Args {
    screenshot: Option<PathBuf>,
    size: (f32, f32),
    scenario: Option<String>,
    theme: Option<bool>,
    models: Option<PathBuf>,
    solver: Option<SolverKind>,
    list_solvers: bool,
    check_solvers: bool,
    files: Vec<PathBuf>,
}

fn usage() -> ! {
    eprintln!(
        "usage: coxeter [--screenshot OUT.png] [--size WxH] [--scenario ID] [--dark|--light] [--models DIR] [--solver highs|gurobi|cplex] [--solvers] [--check-solvers] [--version] [files…]"
    );
    std::process::exit(2)
}

fn parse_args() -> Args {
    let mut args = Args {
        screenshot: None,
        size: (1440.0, 900.0),
        scenario: None,
        theme: None,
        models: None,
        solver: None,
        list_solvers: false,
        check_solvers: false,
        files: Vec::new(),
    };
    let mut it = std::env::args_os().skip(1);
    while let Some(arg) = it.next() {
        match arg.to_string_lossy().as_ref() {
            "--version" | "-V" => {
                println!("Coxeter-MFV {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => usage(),
            "--screenshot" => args.screenshot = Some(it.next().map(PathBuf::from).unwrap_or_else(|| usage())),
            "--size" => {
                let s = it.next().unwrap_or_else(|| usage()).to_string_lossy().into_owned();
                let (w, h) = s.split_once(['x', 'X']).unwrap_or_else(|| usage());
                args.size = (w.parse().unwrap_or_else(|_| usage()), h.parse().unwrap_or_else(|_| usage()));
            }
            "--scenario" => args.scenario = Some(it.next().unwrap_or_else(|| usage()).to_string_lossy().into_owned()),
            "--dark" => args.theme = Some(true),
            "--light" => args.theme = Some(false),
            "--models" => args.models = Some(it.next().map(PathBuf::from).unwrap_or_else(|| usage())),
            "--solver" => {
                let name = it.next().unwrap_or_else(|| usage()).to_string_lossy().into_owned();
                args.solver = Some(SolverKind::parse(&name).unwrap_or_else(|| usage()));
            }
            "--solvers" => args.list_solvers = true,
            "--check-solvers" => args.check_solvers = true,
            s if s.starts_with("--") => usage(),
            _ => args.files.push(PathBuf::from(arg)),
        }
    }
    args
}

/// The store, with the bundled files, downloads and their cache.
fn store(args: &Args) -> Store {
    let (assets, _) = match assets::choose(args.models.as_deref()) {
        Ok(a) => a,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };
    let cache = assets::cache_dir();
    let fetcher = CachedFetcher::new(coxeter_analysis::HttpsFetcher::default(), cache.join("downloads"));
    Store::new(assets).with_fetcher(std::sync::Arc::new(fetcher)).with_download_cache(cache.join("zuniga"))
}

fn shell(args: &Args, dialogs: Box<dyn FileDialogs>, remember: bool) -> Shell {
    let mut shell = Shell::new(store(args), dialogs);
    let settings = assets::config_dir().join("settings.json");
    let dark = args.theme.or_else(|| Shell::saved_theme(&settings)).unwrap_or(false);
    shell.store.view.dark = dark;
    let (saved, libraries) = Shell::saved_solver(&settings);
    shell.solvers = Solvers::found(libraries);
    if let Some(kind) = args.solver.or(saved).filter(|k| *k != SolverKind::Highs) {
        if args.solver.is_some() || args.screenshot.is_some() {
            // Asked for by name, or one frame to draw: open it now, so the
            // first solve already runs on it.
            shell.solvers.check_now();
            let info = shell.solvers.info(kind);
            match info.and_then(|i| i.solver.clone()) {
                Some(solver) => shell.store.solver = solver,
                None => eprintln!("{} cannot be used: {}", kind.name(), info.and_then(|i| i.error.clone()).unwrap_or_default()),
            }
        } else {
            // The solver chosen last time: the analyses switch to it once
            // its licence has been checked, in the background.
            shell.solvers.wanted = Some(kind);
            shell.solvers.check();
        }
    }
    if remember {
        shell.settings_path = Some(settings);
    }
    shell.boot(args.scenario.as_deref());
    if !args.files.is_empty() {
        shell.open_paths(&args.files);
    }
    shell
}

/// `--screenshot`: one frame (after background work settles) drawn
/// offscreen, as the JS app would show it.
fn screenshot(args: &Args, out: &std::path::Path) -> Result<(), String> {
    let mut shell = shell(args, Box::new(ScriptedDialogs::new()), false);
    shell.store.wait_for_jobs(Duration::from_secs(300));
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(args.size.0, args.size.1))
        .with_max_steps(64)
        .wgpu()
        .build_eframe(move |_cc| shell);
    for _ in 0..3 {
        let _ = harness.try_run();
        harness.state_mut().store.wait_for_jobs(Duration::from_secs(300));
    }
    let image = harness.render()?;
    image.save(out).map_err(|e| format!("{}: {e}", out.display()))?;
    if let Some(error) = &harness.state().boot_error {
        eprintln!("{error}");
    }
    let missing = coxeter_app::fonts::missing_glyphs(&harness.ctx);
    if !missing.is_empty() {
        eprintln!("No installed font draws: {}", missing.iter().collect::<String>());
    }
    Ok(())
}

/// `--solvers` and `--check-solvers`: the solvers of this computer, and with
/// `check`, the self-test battery on each one that opens. The exit code is 1
/// when a check fails.
fn solvers_report(check: bool) -> i32 {
    let (_, libraries) = Shell::saved_solver(&assets::config_dir().join("settings.json"));
    let mut solvers = Solvers::found(libraries);
    solvers.check_now();
    let mut failed = 0;
    for info in &solvers.infos {
        let state = match (&info.solver, &info.error) {
            (Some(solver), _) => solver.limit().map_or_else(|| "ready".to_string(), |limit| format!("ready, {limit}")),
            (None, Some(error)) => error.clone(),
            (None, None) => "not found".to_string(),
        };
        println!("{:<7} {:<10} {state}", info.kind.name(), info.version.as_deref().unwrap_or("—"));
        if let Some(library) = &info.library {
            println!("        {}", library.display());
        }
        if let (true, Some(solver)) = (check, &info.solver) {
            for c in self_test(solver) {
                let detail = if c.detail.is_empty() { String::new() } else { format!(": {}", c.detail) };
                println!("        {} {}{detail}", if c.passed { "ok  " } else { "FAIL" }, c.name);
                failed += usize::from(!c.passed);
            }
        }
    }
    if check {
        println!("{}", if failed == 0 { "Every check passed.".to_string() } else { format!("{failed} checks failed.") });
    }
    i32::from(failed > 0)
}

/// The brand mark of the header (`.brand-mark`), as the window's icon: the
/// path through four metabolites, in the accent blue.
fn brand_icon() -> egui::IconData {
    const N: usize = 64;
    let k = N as f32 / 34.0;
    let points = [(7.0, 25.0), (14.0, 17.0), (19.0, 8.0), (27.0, 13.0)];
    let radii = [4.0, 3.3, 4.0, 3.0];
    let dist_segment = |px: f32, py: f32, (ax, ay): (f32, f32), (bx, by): (f32, f32)| {
        let (dx, dy) = (bx - ax, by - ay);
        let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        ((px - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
    };
    let mut rgba = vec![0u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            let (px, py) = ((x as f32 + 0.5) / k, (y as f32 + 0.5) / k);
            let mut d = f32::MAX;
            for w in points.windows(2) {
                d = d.min(dist_segment(px, py, w[0], w[1]) - 1.25);
            }
            for (p, r) in points.iter().zip(radii) {
                d = d.min(((px - p.0).powi(2) + (py - p.1).powi(2)).sqrt() - r);
            }
            let alpha = (0.5 - d * k).clamp(0.0, 1.0);
            let i = (y * N + x) * 4;
            rgba[i..i + 4].copy_from_slice(&[0x25, 0x63, 0xeb, (alpha * 255.0) as u8]);
        }
    }
    egui::IconData { rgba, width: N as u32, height: N as u32 }
}

fn main() {
    let args = parse_args();
    if args.list_solvers || args.check_solvers {
        std::process::exit(solvers_report(args.check_solvers));
    }
    if let Some(out) = args.screenshot.clone() {
        if let Err(message) = screenshot(&args, &out) {
            eprintln!("{message}");
            std::process::exit(1);
        }
        return;
    }
    let shell = shell(&args, Box::new(NativeDialogs::default()), true);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Coxeter Multicell Flux Visualizer")
            .with_inner_size([args.size.0, args.size.1])
            .with_min_inner_size([900.0, 560.0])
            .with_icon(brand_icon()),
        // egui feathers its own widgets, but the map is lyon meshes with
        // hard edges: without multisampling every line and glyph is jagged.
        // 4× is the count every wgpu adapter supports.
        multisampling: 4,
        ..Default::default()
    };
    if let Err(error) = eframe::run_native("Coxeter-MFV", options, Box::new(|_cc| Ok(Box::new(shell)))) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
