//! Small widget wrappers shared by panels and plugins.

use eframe::egui;

/// `color_edit_button_rgb` that writes back only on a real edit. egui's own
/// button round-trips the value through HSV every frame, which nudges the
/// floats (0.45 -> 0.44999972) and marks an untouched scene as unsaved.
pub fn color_rgb(ui: &mut egui::Ui, rgb: &mut [f32; 3]) -> egui::Response {
    let mut edit = *rgb;
    let response = ui.color_edit_button_rgb(&mut edit);
    if response.changed() {
        *rgb = edit;
    }
    response
}

/// [`color_rgb`] for `color_edit_button_rgba_unmultiplied`.
pub fn color_rgba(ui: &mut egui::Ui, rgba: &mut [f32; 4]) -> egui::Response {
    let mut edit = *rgba;
    let response = ui.color_edit_button_rgba_unmultiplied(&mut edit);
    if response.changed() {
        *rgba = edit;
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untouched_colours_stay_bit_exact() {
        let ctx = egui::Context::default();
        let mut rgb = [0.45, 0.3, 0.1];
        let mut rgba = [0.45, 0.3, 0.1, 0.7];
        let (rgb0, rgba0) = (rgb, rgba);
        for _ in 0..3 {
            ctx.run_ui(egui::RawInput::default(), |ui| {
                color_rgb(ui, &mut rgb);
                color_rgba(ui, &mut rgba);
            })
            .textures_delta
            .clear();
        }
        assert_eq!(rgb, rgb0);
        assert_eq!(rgba, rgba0);
    }
}
