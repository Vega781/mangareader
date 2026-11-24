//! Crisp vector icons drawn with the egui painter (no tofu glyphs).
use eframe::egui::{self, Color32, Sense, Ui, Vec2};

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    /// ◄ (page/chapter back)
    Prev,
    /// ►
    Next,
    /// |◄ skip to start
    First,
    /// ►| skip to end
    Last,
    /// ◄◄ double (RTL indicator)
    DblPrev,
    /// ►►
    DblNext,
    /// bookmark star, outline
    Star,
    /// bookmark star, filled
    StarFilled,
    /// download arrow into tray
    Download,
    /// checkmark
    Check,
}

fn tri(ui: &mut Ui, apex: egui::Pos2, half_h: f32, back: f32, color: Color32) {
    // apex: tip position; triangle points towards +x if back>0 else -x
    let c = apex;
    let p = ui.painter();
    let pts = vec![
        egui::pos2(c.x, c.y),
        egui::pos2(c.x - back, c.y - half_h),
        egui::pos2(c.x - back, c.y + half_h),
    ];
    p.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
}

pub fn draw(ui: &mut Ui, icon: Icon, rect: egui::Rect, color: Color32) {
    let p = ui.painter();
    let c = rect.center();
    let s = rect.height().min(rect.width()) * 0.42; // icon half-size
    let stroke = egui::Stroke::new(1.8, color);
    match icon {
        Icon::Prev | Icon::Next => {
            let dir = if matches!(icon, Icon::Next) { 1.0 } else { -1.0 };
            tri(
                ui,
                c + egui::vec2(dir * s * 0.9, 0.0),
                s * 0.85,
                dir * s * 1.6,
                color,
            );
        }
        Icon::DblPrev | Icon::DblNext => {
            let dir = if matches!(icon, Icon::DblNext) { 1.0 } else { -1.0 };
            tri(
                ui,
                c + egui::vec2(dir * s * 1.35, 0.0),
                s * 0.75,
                dir * s * 1.2,
                color,
            );
            tri(
                ui,
                c + egui::vec2(dir * s * 0.15, 0.0),
                s * 0.75,
                dir * s * 1.2,
                color,
            );
        }
        Icon::First | Icon::Last => {
            let dir = if matches!(icon, Icon::Last) { 1.0 } else { -1.0 };
            // bar
            let bar_x = c.x - dir * s * 1.15;
            p.line_segment(
                [
                    egui::pos2(bar_x, c.y - s * 0.95),
                    egui::pos2(bar_x, c.y + s * 0.95),
                ],
                stroke,
            );
            tri(
                ui,
                c + egui::vec2(dir * s * 0.55, 0.0),
                s * 0.85,
                dir * s * 1.35,
                color,
            );
        }
        Icon::Star | Icon::StarFilled => {
            let r_out = s * 1.15;
            let r_in = r_out * 0.42;
            let mut pts = Vec::with_capacity(10);
            for i in 0..10 {
                let r = if i % 2 == 0 { r_out } else { r_in };
                let a = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::PI / 5.0;
                pts.push(egui::pos2(
                    c.x + (a.cos() as f32) * r,
                    c.y + (a.sin() as f32) * r,
                ));
            }
            let fill = if matches!(icon, Icon::StarFilled) {
                color
            } else {
                Color32::TRANSPARENT
            };
            let stroke = if matches!(icon, Icon::StarFilled) {
                egui::Stroke::NONE
            } else {
                egui::Stroke::new(1.6, color)
            };
            p.add(egui::Shape::convex_polygon(pts, fill, stroke));
        }
        Icon::Download => {
            // vertical arrow down
            ui.painter().line_segment([c - egui::vec2(0.0, s), c + egui::vec2(0.0, s * 0.35)], stroke);
            tri(
                ui,
                c + egui::vec2(0.0, s * 0.75),
                s * 0.55,
                -s * 0.95,
                color,
            );
            // tray
            ui.painter().line_segment(
                [
                    c + egui::vec2(-s * 0.95, s * 0.55),
                    c + egui::vec2(-s * 0.95, s),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [c + egui::vec2(-s * 0.95, s), c + egui::vec2(s * 0.95, s)],
                stroke,
            );
            ui.painter().line_segment(
                [c + egui::vec2(s * 0.95, s), c + egui::vec2(s * 0.95, s * 0.55)],
                stroke,
            );
        }
        Icon::Check => {
            p.add(egui::Shape::line(
                vec![
                    c + egui::vec2(-s * 0.9, s * 0.05),
                    c + egui::vec2(-s * 0.2, s * 0.7),
                    c + egui::vec2(s * 0.9, -s * 0.7),
                ],
                egui::Stroke::new(2.2, color),
            ));
        }
    }
}

/// Icon-only button.
pub fn button(ui: &mut Ui, icon: Icon, tip: &str) -> egui::Response {
    let sz = ui.spacing().interact_size.y * 0.85;
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(sz), Sense::click());
    if resp.hovered() {
        let bg = ui.visuals().widgets.hovered.bg_fill;
        ui.painter().rect_filled(rect, 4.0, bg);
    }
    let fg = if resp.hovered() {
        ui.visuals().widgets.hovered.fg_stroke.color
    } else {
        ui.visuals().widgets.inactive.fg_stroke.color
    };
    draw(ui, icon, rect, fg);
    resp.on_hover_text(tip)
}

/// Icon + label button, drawn manually so glyphs are always crisp.
pub fn button_with_text(ui: &mut Ui, icon: Icon, text: &str, selected: bool) -> egui::Response {
    let icon_sz = ui.spacing().interact_size.y * 0.75;
    let font = egui::FontId::proportional(14.0);
    let fg_normal = if selected {
        ui.visuals().widgets.active.fg_stroke.color
    } else {
        ui.visuals().widgets.inactive.fg_stroke.color
    };
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), fg_normal);
    let pad = 9.0_f32;
    let w = pad + icon_sz + 6.0 + galley.size().x + pad;
    let h = ui.spacing().interact_size.y.max(24.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    if resp.hovered() || selected {
        let bg = if resp.hovered() {
            ui.visuals().widgets.hovered.bg_fill
        } else {
            ui.visuals().widgets.inactive.bg_fill
        };
        ui.painter().rect_filled(rect, 5.0, bg);
    }
    let fg = if resp.hovered() {
        ui.visuals().widgets.hovered.fg_stroke.color
    } else {
        fg_normal
    };
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + pad, rect.center().y - icon_sz / 2.0),
        Vec2::splat(icon_sz),
    );
    draw(ui, icon, icon_rect, fg);
    ui.painter().galley(
        egui::pos2(
            rect.left() + pad + icon_sz + 6.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        fg,
    );
    resp
}
