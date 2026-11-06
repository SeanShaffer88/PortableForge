//! Static VaporGrid tread-plate texture.

use eframe::egui;

pub(super) fn paint(ui: &egui::Ui) {
    let rect = ui.clip_rect();
    if !rect.is_finite() || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }

    // Fit a bounded mesh of square tiles to the visible area.
    let pitch = 42.0_f32.max(rect.width() / 32.0).max(rect.height() / 24.0);
    let columns = (rect.width() / pitch).ceil() as usize;
    let rows = (rect.height() / pitch).ceil() as usize;
    let light = egui::Color32::from_rgb(27, 29, 35);
    let shade = egui::Color32::from_rgb(17, 18, 23);
    let seam = egui::Color32::from_rgb(23, 25, 31);
    let mut mesh = egui::Mesh::default();
    for row in 0..rows {
        for column in 0..columns {
            let origin = rect.min + egui::vec2(column as f32 * pitch, row as f32 * pitch);
            mesh.add_colored_rect(
                egui::Rect::from_min_size(origin, egui::vec2(pitch, 0.7)),
                seam,
            );
            mesh.add_colored_rect(
                egui::Rect::from_min_size(origin, egui::vec2(0.7, pitch)),
                seam,
            );
            for (x, y, direction) in [
                (0.25, 0.25, 1.0),
                (0.75, 0.25, -1.0),
                (0.25, 0.75, -1.0),
                (0.75, 0.75, 1.0),
            ] {
                let center = origin + egui::vec2(x * pitch, y * pitch);
                let axis = egui::vec2(5.2, 5.2 * direction);
                let cross = egui::vec2(-1.25 * direction, 1.25);
                let ridge = mesh.vertices.len() as u32;
                mesh.colored_vertex(center - axis, light);
                mesh.colored_vertex(center - cross, light);
                mesh.colored_vertex(center + axis, light);
                mesh.colored_vertex(center + cross, shade);
                mesh.add_triangle(ridge, ridge + 1, ridge + 2);
                mesh.add_triangle(ridge, ridge + 2, ridge + 3);
            }
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
}
