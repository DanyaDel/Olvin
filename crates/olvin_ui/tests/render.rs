//! Off-screen renders for eyeballing the themes and the logo. Needs a GPU
//! (or llvmpipe), so it's ignored by default:
//!
//! ```sh
//! __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json WGPU_BACKEND=gl \
//!     cargo test -p olvin_ui --test render -- --ignored
//! ```
//!
//! PNGs land in `target/tmp/olvin_ui/` (or `$OLVIN_UI_RENDER_DIR`).

use egui::{Frame, Margin, RichText};
use egui_kittest::Harness;
use olvin_ui::{ThemeKind, anim, logo};

fn out_dir() -> std::path::PathBuf {
    let dir = std::env::var_os("OLVIN_UI_RENDER_DIR")
        .map(Into::into)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("olvin_ui"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sample(ui: &mut egui::Ui, kind: ThemeKind) {
    olvin_ui::apply(ui.ctx(), kind);
    // New fonts (the bold family) arrive on the next pass.
    if ui.ctx().cumulative_pass_nr() == 0 {
        return;
    }
    let p = olvin_ui::palette(ui.ctx());
    let now = ui.input(|i| i.time);
    Frame::new()
        .fill(p.bg)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            // Top bar.
            Frame::new()
                .fill(p.bg)
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        logo::wordmark(ui, 17.0);
                        ui.add_space(8.0);
                        for (i, t) in ["File", "Edit", "Window"].into_iter().enumerate() {
                            ui.add(anim::button(ui, olvin_ui::menu_title(t), i == 1));
                        }
                        ui.add(anim::button(ui, "▶", true));
                    });
                });
            ui.add_space(8.0);
            Frame::new()
                .fill(p.surface)
                .corner_radius(olvin_ui::RADIUS_MD)
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.label(olvin_ui::caps("Transform"));
                    ui.label(RichText::new(kind.label()).heading());
                    ui.label("Main text: the quick brown fox.");
                    ui.weak("Muted: secondary labels.");
                    ui.horizontal(|ui| {
                        ui.colored_label(p.warn, "warning");
                        ui.colored_label(p.error, "error");
                        ui.colored_label(p.info, "info");
                        ui.colored_label(p.accent, "● playing");
                    });
                    ui.horizontal(|ui| {
                        let _ = ui.button("Secondary");
                        let _ = ui.selectable_label(true, "Selected");
                        let mut s = String::from("text field");
                        ui.text_edit_singleline(&mut s);
                    });
                    let mut on = true;
                    ui.checkbox(&mut on, "Checkbox");
                });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                logo::mark(ui, 32.0);
                ui.add_space(12.0);
                logo::mark(ui, 16.0);
                ui.add_space(12.0);
                logo::wordmark(ui, 48.0);
            });
            ui.add_space(10.0);
            // Intro frozen mid-typing and mid-glide.
            ui.horizontal(|ui| {
                logo::wordmark_intro(ui, 30.0, now - 0.75);
                ui.add_space(16.0);
                logo::wordmark_intro(ui, 30.0, now - 1.18);
            });
        });
}

#[test]
#[ignore = "needs a GPU or llvmpipe"]
fn render_themes_and_logo() {
    let dir = out_dir();
    for kind in ThemeKind::ALL {
        let mut harness = Harness::builder()
            .with_size([620.0, 420.0])
            .with_pixels_per_point(1.5)
            .wgpu()
            .build_ui(move |ui| sample(ui, kind));
        harness.run_steps(4);
        let image = harness.render().expect("render");
        let path = dir.join(format!("theme-{kind:?}.png").to_lowercase());
        image.save(&path).unwrap();
        println!("{}", path.display());
    }
}
