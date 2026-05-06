use adw::prelude::*;
use qrcode::{EcLevel, QrCode};

pub fn build(text: &str, size: i32) -> gtk::Widget {
    if text.trim().is_empty() {
        let label = gtk::Label::new(Some(""));
        label.add_css_class("dim-label");
        label.set_size_request(size, size);
        return label.upcast();
    }

    let area = gtk::DrawingArea::new();
    area.set_content_width(size);
    area.set_content_height(size);
    area.set_size_request(size, size);
    area.set_halign(gtk::Align::Center);

    let code = [EcLevel::M, EcLevel::L]
        .into_iter()
        .find_map(|level| QrCode::with_error_correction_level(text.as_bytes(), level).ok());

    let Some(code) = code else {
        let label = gtk::Label::new(Some("Code unavailable"));
        label.add_css_class("dim-label");
        label.set_halign(gtk::Align::Center);
        return label.upcast();
    };

    let modules = code.width();
    let bits: Vec<bool> = code
        .to_colors()
        .into_iter()
        .map(|color| matches!(color, qrcode::Color::Dark))
        .collect();

    area.set_draw_func(move |_, ctx, width, height| {
        let dim = width.min(height) as f64;
        ctx.set_source_rgb(1.0, 1.0, 1.0);
        let _ = ctx.paint();

        if modules == 0 {
            return;
        }

        let quiet = 4usize;
        let total = modules + quiet * 2;
        let scale = dim / total as f64;
        let offset_x = (width as f64 - dim) / 2.0;
        let offset_y = (height as f64 - dim) / 2.0;

        ctx.set_source_rgb(0.0, 0.0, 0.0);
        for y in 0..modules {
            for x in 0..modules {
                if bits[y * modules + x] {
                    let px = offset_x + (quiet + x) as f64 * scale;
                    let py = offset_y + (quiet + y) as f64 * scale;
                    ctx.rectangle(px, py, scale, scale);
                }
            }
        }
        let _ = ctx.fill();
    });

    area.upcast()
}
