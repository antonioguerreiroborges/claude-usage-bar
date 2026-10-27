//! Tamanho da janela, independente de DPI.
//!
//! A largura é guardada em pontos lógicos. Em pixels físicos ela crescia 25%
//! a cada abertura num monitor de 125%: a janela nasce na escala do monitor
//! principal, e ao chegar no outro monitor o winit preserva o tamanho lógico
//! (px novo = px / escala antiga × escala nova), que então era salvo de novo.

pub const HEIGHT: f32 = 28.0;
pub const MIN_WIDTH: f32 = 150.0;
pub const MAX_WIDTH: f32 = 1600.0;
pub const DEFAULT_WIDTH: f32 = 200.0;
/// Diferença abaixo da qual a largura já está certa.
const TOLERANCE: f32 = 1.0;

pub fn clamp_width(pt: f32) -> f32 {
    if pt.is_finite() {
        pt.clamp(MIN_WIDTH, MAX_WIDTH)
    } else {
        DEFAULT_WIDTH
    }
}

/// Largura a impor agora: `None` se a atual já está certa ou se o usuário
/// está redimensionando (aí a largura nova é dele).
pub fn width_fix(desired_pt: f32, current_pt: f32, user_resizing: bool) -> Option<f32> {
    if user_resizing || (desired_pt - current_pt).abs() < TOLERANCE {
        None
    } else {
        Some(desired_pt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE_DPI: u32 = 96;

    /// O que o egui reporta (pontos) para uma largura física num DPI.
    fn px_to_pt(px: i32, dpi: u32) -> f32 {
        px as f32 * BASE_DPI as f32 / dpi.max(1) as f32
    }

    fn pt_to_px(pt: f32, dpi: u32) -> i32 {
        (pt * dpi as f32 / BASE_DPI as f32).round() as i32
    }

    /// A regra do winit 0.30 em `WM_DPICHANGED`: mantém o tamanho lógico.
    fn winit_dpi_change(px: i32, old_dpi: u32, new_dpi: u32) -> i32 {
        (px as f64 / old_dpi as f64 * new_dpi as f64).round() as i32
    }

    /// Uma abertura: a janela nasce em `start_dpi` com a largura
    /// restaurada, vai para o monitor salvo em `monitor_dpi`, e a largura
    /// física resultante é devolvida.
    fn open_on(monitor_dpi: u32, start_dpi: u32, created_px: i32) -> i32 {
        winit_dpi_change(created_px, start_dpi, monitor_dpi)
    }

    #[test]
    fn restart_on_other_monitor_keeps_width() {
        // Principal a 100%, barra salva no monitor de 125%.
        let (primary, other) = (96, 120);
        let mut saved = DEFAULT_WIDTH;
        for _ in 0..10 {
            let created = pt_to_px(saved, primary);
            let landed = open_on(other, primary, created);
            saved = px_to_pt(landed, other);
            assert!((saved - DEFAULT_WIDTH).abs() < 0.5, "largura salva derivou para {saved}");
        }
        // E de volta ao principal, sem acumular nada.
        let landed = open_on(primary, other, pt_to_px(saved, other));
        assert!((px_to_pt(landed, primary) - DEFAULT_WIDTH).abs() < 0.5);
    }

    /// Controle: como era antes (pixels físicos salvos e reaplicados na
    /// escala do monitor principal). Mostra que o teste acima pega o bug.
    #[test]
    fn physical_persistence_grows() {
        let (primary, other) = (96, 120);
        let mut saved_px = pt_to_px(DEFAULT_WIDTH, other); // 250
        let first = saved_px;
        for _ in 0..3 {
            saved_px = open_on(other, primary, saved_px);
        }
        assert!(saved_px as f32 > first as f32 * 1.9, "{first} -> {saved_px}");
    }

    #[test]
    fn px_pt_round_trip() {
        for dpi in [96, 120, 144, 192] {
            for pt in [150.0_f32, 200.0, 333.0, 1600.0] {
                let back = px_to_pt(pt_to_px(pt, dpi), dpi);
                assert!((back - pt).abs() < 0.5, "{pt} pt @ {dpi} dpi -> {back}");
            }
        }
        assert_eq!(pt_to_px(200.0, 120), 250);
        assert_eq!(px_to_pt(250, 120), 200.0);
    }

    #[test]
    fn width_fix_rules() {
        assert_eq!(width_fix(200.0, 312.5, false), Some(200.0));
        assert_eq!(width_fix(200.0, 200.4, false), None);
        assert_eq!(width_fix(200.0, 312.0, true), None);
    }

    #[test]
    fn clamp_width_limits() {
        assert_eq!(clamp_width(779.0), 779.0);
        assert_eq!(clamp_width(50.0), MIN_WIDTH);
        assert_eq!(clamp_width(99_999.0), MAX_WIDTH);
        assert_eq!(clamp_width(f32::NAN), DEFAULT_WIDTH);
    }
}
