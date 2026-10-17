//! 画面全体の見た目。色・余白・角丸はここだけで決め、各画面へ直書きしない。
//! 旧Python版の意匠は踏襲せず、落ち着いた配色と大きめの余白で作り直している。
use eframe::egui::{self, Color32, CornerRadius, FontId, Stroke, TextStyle};

pub const BG: Color32 = Color32::from_rgb(17, 19, 24);
pub const SURFACE: Color32 = Color32::from_rgb(24, 27, 33);
pub const CARD: Color32 = Color32::from_rgb(31, 35, 43);
pub const CARD_HOVER: Color32 = Color32::from_rgb(40, 45, 56);
pub const BORDER: Color32 = Color32::from_rgb(48, 54, 66);
pub const TEXT: Color32 = Color32::from_rgb(230, 233, 239);
pub const MUTED: Color32 = Color32::from_rgb(137, 146, 163);
pub const ACCENT: Color32 = Color32::from_rgb(86, 204, 178);
pub const ACCENT_TEXT: Color32 = Color32::from_rgb(10, 28, 24);
pub const DANGER: Color32 = Color32::from_rgb(235, 117, 108);
pub const WARN: Color32 = Color32::from_rgb(230, 184, 92);
pub const OK: Color32 = Color32::from_rgb(98, 210, 142);

pub const RADIUS: u8 = 10;
pub const GAP: f32 = 12.0;

pub fn apply(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        v.panel_fill = BG;
        v.window_fill = SURFACE;
        v.extreme_bg_color = Color32::from_rgb(12, 14, 18);
        v.faint_bg_color = CARD;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.window_corner_radius = CornerRadius::same(RADIUS + 2);
        v.menu_corner_radius = CornerRadius::same(RADIUS);
        v.hyperlink_color = ACCENT;
        v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.override_text_color = Some(TEXT);
        let radius = CornerRadius::same(8);
        for (w, fill) in [
            (&mut v.widgets.inactive, CARD),
            (&mut v.widgets.hovered, CARD_HOVER),
            (&mut v.widgets.active, CARD_HOVER),
            (&mut v.widgets.open, CARD_HOVER),
        ] {
            w.bg_fill = fill;
            w.weak_bg_fill = fill;
            w.corner_radius = radius;
            w.bg_stroke = Stroke::new(1.0, BORDER);
        }
        v.widgets.inactive.bg_stroke = Stroke::NONE;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        v.widgets.noninteractive.corner_radius = radius;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.6));
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.window_margin = egui::Margin::same(16);
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(15.0));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(15.0));
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(12.5));
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::proportional(24.0));
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::monospace(14.0));
    });
}

/// 主操作のボタン。画面に1つだけ置く。
pub fn primary(text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text).color(ACCENT_TEXT).strong())
        .fill(ACCENT)
        .stroke(Stroke::NONE)
}

pub fn danger(text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text).color(DANGER)).fill(Color32::TRANSPARENT)
}

pub fn muted(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into()).color(MUTED).size(13.0)
}

pub fn title(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into()).size(18.0).strong()
}

pub fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(egui::Margin::same(14))
}

/// 小さな状態表示（名前付きの色つき丸）。
pub fn dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// 角丸の小さな札。接続方式などの補足に使う。
pub fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.18))
        .corner_radius(CornerRadius::same(20))
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(12.0).color(color));
        });
}

/// 2〜4択の切替。選択中だけ塗る。
pub fn segmented<T: PartialEq + Clone>(
    ui: &mut egui::Ui,
    value: &mut T,
    items: &[(T, &str)],
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (item, label) in items {
            let selected = *value == *item;
            let text = if selected {
                egui::RichText::new(*label).color(ACCENT_TEXT).strong()
            } else {
                egui::RichText::new(*label)
            };
            let button = egui::Button::new(text).fill(if selected { ACCENT } else { CARD });
            if ui.add(button).clicked() && !selected {
                *value = item.clone();
                changed = true;
            }
        }
    });
    changed
}

/// 名前から決まる穴埋めの色。同じ接続先はいつも同じ色になる。
pub fn avatar_color(name: &str) -> Color32 {
    const COLORS: [Color32; 6] = [
        Color32::from_rgb(86, 204, 178),
        Color32::from_rgb(120, 160, 255),
        Color32::from_rgb(230, 150, 110),
        Color32::from_rgb(200, 130, 230),
        Color32::from_rgb(230, 184, 92),
        Color32::from_rgb(235, 117, 108),
    ];
    let sum = name
        .bytes()
        .fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize));
    COLORS[sum % COLORS.len()]
}

pub fn avatar(ui: &mut egui::Ui, name: &str, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let color = avatar_color(name);
    ui.painter().rect_filled(
        rect,
        CornerRadius::same((size / 3.0) as u8),
        color.gamma_multiply(0.22),
    );
    let initial = name
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or("?".into());
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        FontId::proportional(size * 0.5),
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn avatar_color_is_stable_per_name() {
        assert_eq!(avatar_color("本番サーバー"), avatar_color("本番サーバー"));
    }
}
