//! Textos e cores da barra.

use chrono::Duration;
use eframe::egui::Color32;

pub const LOW: Color32 = Color32::from_rgb(0x2a, 0x78, 0xd6);
pub const MID: Color32 = Color32::from_rgb(0xfa, 0xb2, 0x19);
pub const HIGH: Color32 = Color32::from_rgb(0xd0, 0x3b, 0x3b);

pub const INK_LIGHT: Color32 = Color32::from_rgb(0xf0, 0xee, 0xe6);
pub const INK_DARK: Color32 = Color32::from_rgb(0x20, 0x20, 0x1e);

/// Mesmos degraus da paleta dinâmica do widget antigo.
pub fn level_color(pct: f64) -> Color32 {
    if pct >= 85.0 {
        HIGH
    } else if pct >= 50.0 {
        MID
    } else {
        LOW
    }
}

/// Texto escuro sobre preenchimentos claros (o âmbar), claro sobre o resto.
pub fn ink_on(fill: Color32) -> Color32 {
    let lin = |c: u8| {
        let c = c as f64 / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let l = 0.2126 * lin(fill.r()) + 0.7152 * lin(fill.g()) + 0.0722 * lin(fill.b());
    if l > 0.35 {
        INK_DARK
    } else {
        INK_LIGHT
    }
}

pub fn percent(pct: f64) -> String {
    format!("{:.0}%", pct.clamp(0.0, 999.0))
}

/// "2d 5h", "2h 05min", "14min" ou, no último minuto, "42s".
pub fn time_left(d: Duration) -> String {
    let secs = d.num_seconds().max(0);
    if secs < 60 {
        return format!("{secs}s");
    }
    let mins = (secs + 59) / 60;
    if mins >= 24 * 60 {
        format!("{}d {}h", mins / 1440, (mins % 1440) / 60)
    } else if mins >= 60 {
        format!("{}h {:02}min", mins / 60, mins % 60)
    } else {
        format!("{mins}min")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_steps() {
        assert_eq!(level_color(0.0), LOW);
        assert_eq!(level_color(49.9), LOW);
        assert_eq!(level_color(50.0), MID);
        assert_eq!(level_color(84.9), MID);
        assert_eq!(level_color(85.0), HIGH);
        assert_eq!(level_color(100.0), HIGH);
    }

    #[test]
    fn ink_contrast() {
        assert_eq!(ink_on(MID), INK_DARK);
        assert_eq!(ink_on(LOW), INK_LIGHT);
        assert_eq!(ink_on(HIGH), INK_LIGHT);
    }

    #[test]
    fn time_left_forms() {
        assert_eq!(time_left(Duration::seconds(42)), "42s");
        assert_eq!(time_left(Duration::seconds(-5)), "0s");
        assert_eq!(time_left(Duration::minutes(14)), "14min");
        assert_eq!(time_left(Duration::seconds(14 * 60 + 1)), "15min");
        assert_eq!(time_left(Duration::minutes(134)), "2h 14min");
        assert_eq!(time_left(Duration::minutes(65)), "1h 05min");
        assert_eq!(time_left(Duration::hours(53)), "2d 5h");
    }

    #[test]
    fn percent_rounds() {
        assert_eq!(percent(61.6), "62%");
        assert_eq!(percent(-3.0), "0%");
    }
}
