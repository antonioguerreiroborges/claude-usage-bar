//! A janela: uma barra fina com a porcentagem da janela de 5 horas e o tempo
//! até o reset.

use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration as StdDuration, Instant};

use chrono::{Local, Utc};
use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Rect, Sense, ViewportCommand};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::fmt;
use crate::geom;
use crate::store::{AccountInfo, Config, LastUsage};
use crate::win::{self, MenuItem};
use crate::worker::{self, Cmd, Snapshot, Status};

const BG: Color32 = Color32::from_rgb(0x1f, 0x1f, 0x1e);
const TRACK: Color32 = Color32::from_rgb(0x2c, 0x2c, 0x2a);
const MUTED: Color32 = Color32::from_rgb(0xa8, 0xa6, 0x9e);
const WARN: Color32 = Color32::from_rgb(0xff, 0x8a, 0x65);
/// Largura a partir da qual cabe também o horário do reset.
const WIDE: f32 = 230.0;
const EDGE: f32 = 8.0;
const TOPMOST_EVERY: StdDuration = StdDuration::from_secs(2);

const ID_REFRESH: u32 = 1;
const ID_USE_LIVE: u32 = 2;
const ID_AUTOSTART: u32 = 3;
const ID_QUIT: u32 = 4;

pub struct App {
    cfg: Config,
    tx: Sender<Cmd>,
    rx: Receiver<Snapshot>,
    status: Status,
    usage: Option<LastUsage>,
    warn: Option<String>,
    account: Option<AccountInfo>,
    hwnd: Option<isize>,
    topmost_at: Instant,
    /// A largura que a barra deve ter, em pontos. Só muda quando o usuário
    /// redimensiona pela borda.
    width_pt: f32,
    resizing: bool,
}

enum View {
    Message(&'static str),
    Bar {
        pct: f64,
        right: String,
    },
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: Config) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let (tx, rx) = worker::spawn(cc.egui_ctx.clone(), cfg.refresh_secs);
        Self {
            usage: cfg.last.clone(),
            account: cfg.account.clone(),
            width_pt: geom::clamp_width(cfg.width_pt.unwrap_or(geom::DEFAULT_WIDTH)),
            resizing: false,
            cfg,
            tx,
            rx,
            status: Status::Loading,
            warn: None,
            hwnd: None,
            topmost_at: Instant::now(),
        }
    }

    fn drain(&mut self) {
        let mut dirty = false;
        while let Ok(s) = self.rx.try_recv() {
            self.status = s.status;
            self.warn = s.warn;
            if s.account.is_some() && s.account != self.cfg.account {
                self.cfg.account = s.account.clone();
                dirty = true;
            }
            if s.account.is_some() {
                self.account = s.account;
            }
            if let Some(u) = s.usage {
                self.cfg.last = Some(u.clone());
                self.usage = Some(u);
                dirty = true;
            }
        }
        if dirty {
            self.cfg.save();
        }
    }

    /// No primeiro frame: estilo de janela de ferramenta e a posição salva.
    fn ensure_window(&mut self, frame: &eframe::Frame) {
        if self.hwnd.is_none() {
            let Ok(handle) = frame.window_handle() else {
                return;
            };
            let RawWindowHandle::Win32(h) = handle.as_raw() else {
                return;
            };
            let hwnd = h.hwnd.get();
            win::make_tool_window(hwnd);
            win::place(hwnd, self.cfg.x, self.cfg.y);
            self.hwnd = Some(hwnd);
            self.topmost_at = Instant::now();
        }
        if let Some(hwnd) = self.hwnd {
            if self.topmost_at.elapsed() >= TOPMOST_EVERY {
                win::keep_topmost(hwnd);
                self.topmost_at = Instant::now();
            }
        }
    }

    /// Mantém a largura desejada. Ao fim de um redimensionamento pela borda,
    /// a largura nova passa a ser a desejada; fora disso, qualquer desvio
    /// (mudança de DPI, peculiaridade do winit) é desfeito.
    fn enforce_width(&mut self, ctx: &egui::Context) {
        if self.hwnd.is_none() {
            return;
        }
        let Some(current) = ctx.input(|i| i.viewport().inner_rect).map(|r| r.width()) else {
            return;
        };
        if self.resizing && !win::left_button_down() {
            self.resizing = false;
            self.width_pt = geom::clamp_width(current);
            if self.cfg.width_pt != Some(self.width_pt) {
                self.cfg.width_pt = Some(self.width_pt);
                self.cfg.save();
            }
            return;
        }
        if let Some(w) = geom::width_fix(self.width_pt, current, self.resizing) {
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(w, geom::HEIGHT)));
        }
    }

    /// Salva a posição quando muda, depois que o mouse é solto.
    fn track_position(&mut self) {
        let Some(hwnd) = self.hwnd else {
            return;
        };
        if win::left_button_down() {
            return;
        }
        let Some((x, y, _, _)) = win::window_rect(hwnd) else {
            return;
        };
        if x <= -30000 {
            return;
        }
        if self.cfg.x != Some(x) || self.cfg.y != Some(y) {
            self.cfg.x = Some(x);
            self.cfg.y = Some(y);
            self.cfg.save();
        }
    }

    fn view(&self, wide: bool) -> View {
        match (self.status, &self.usage) {
            (Status::NoLogin, _) => View::Message("Faça login no Claude Code"),
            (Status::Expired, _) => View::Message("Login expirado"),
            (Status::NoPlan, _) => View::Message("Sem plano ativo"),
            (_, None) if self.warn.is_some() => View::Message("Sem conexão"),
            (_, None) => View::Message("Carregando..."),
            (_, Some(u)) => {
                let now = Utc::now();
                let mut right = match u.resets_at {
                    // O reset passou e ainda não chegou leitura nova: sessão zerada.
                    Some(r) if r <= now => "sessão nova".to_owned(),
                    Some(r) if wide => format!(
                        "{} · {}",
                        fmt::time_left(r - now),
                        r.with_timezone(&Local).format("%H:%M")
                    ),
                    Some(r) => fmt::time_left(r - now),
                    None => "sem uso".to_owned(),
                };
                let pct = match u.resets_at {
                    Some(r) if r <= now => 0.0,
                    _ => u.utilization,
                };
                if self.warn.is_some() {
                    right = format!("! {right}");
                }
                View::Bar { pct, right }
            }
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter().clone();
        painter.rect_filled(rect, 7.0, BG);
        let bar = rect.shrink2(egui::vec2(5.0, 5.0));
        painter.rect_filled(bar, 4.0, TRACK);
        let font = FontId::proportional(12.0);

        match self.view(rect.width() >= WIDE) {
            View::Message(text) => {
                let color = if self.status == Status::Loading { MUTED } else { WARN };
                painter.text(bar.center(), Align2::CENTER_CENTER, text, font, color);
            }
            View::Bar { pct, right } => {
                let fill_color = fmt::level_color(pct);
                let mut fill_w = bar.width() * (pct.clamp(0.0, 100.0) / 100.0) as f32;
                if pct > 0.0 {
                    fill_w = fill_w.max(4.0);
                }
                let fill = Rect::from_min_size(bar.min, egui::vec2(fill_w, bar.height()));
                painter.rect_filled(fill, 4.0, fill_color);

                // A cor de cada texto depende do que está embaixo dele.
                let ink = |x0: f32, x1: f32| {
                    if fill.right() >= (x0 + x1) / 2.0 {
                        fmt::ink_on(fill_color)
                    } else {
                        fmt::INK_LIGHT
                    }
                };
                let left = fmt::percent(pct);
                let lw = painter.layout_no_wrap(left.clone(), font.clone(), Color32::WHITE).size().x;
                let lx = bar.left() + 6.0;
                painter.text(
                    egui::pos2(lx, bar.center().y),
                    Align2::LEFT_CENTER,
                    left,
                    font.clone(),
                    ink(lx, lx + lw),
                );
                let rw = painter.layout_no_wrap(right.clone(), font.clone(), Color32::WHITE).size().x;
                let rx = bar.right() - 6.0;
                painter.text(
                    egui::pos2(rx, bar.center().y),
                    Align2::RIGHT_CENTER,
                    right,
                    font,
                    ink(rx - rw, rx),
                );
            }
        }

        let hover = ui.input(|i| i.pointer.hover_pos());
        let on_edge = hover.is_some_and(|p| p.x >= rect.right() - EDGE);
        if on_edge {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        if ui.input(|i| i.pointer.primary_pressed()) {
            let cmd = if on_edge {
                self.resizing = true;
                ViewportCommand::BeginResize(egui::viewport::ResizeDirection::East)
            } else {
                ViewportCommand::StartDrag
            };
            ui.ctx().send_viewport_cmd(cmd);
        }
        if resp.secondary_clicked() {
            self.menu(ui.ctx());
        }
    }

    fn menu(&mut self, ctx: &egui::Context) {
        let Some(hwnd) = self.hwnd else {
            return;
        };
        let acc = self.account.clone().unwrap_or_default();
        let email = acc.email.unwrap_or_else(|| "Conta não identificada".into());
        let plan = acc.plan.map(|p| format!("Plano {p}"));
        let reset = self.usage.as_ref().and_then(|u| u.resets_at).map(|r| {
            format!(
                "Reset às {} (em {})",
                r.with_timezone(&Local).format("%H:%M"),
                fmt::time_left(r - Utc::now())
            )
        });
        let updated = self.usage.as_ref().map(|u| {
            format!("Atualizado às {}", u.fetched_at.with_timezone(&Local).format("%H:%M"))
        });
        let warn = self.warn.clone();
        let autostart = win::autostart_enabled();

        let mut items = vec![MenuItem::Info(&email)];
        for line in [&plan, &reset, &updated, &warn].into_iter().flatten() {
            items.push(MenuItem::Info(line));
        }
        items.extend([
            MenuItem::Separator,
            MenuItem::Action { id: ID_REFRESH, text: "Atualizar agora", checked: false },
            MenuItem::Action {
                id: ID_USE_LIVE,
                text: "Usar a conta atual do Claude Code",
                checked: false,
            },
            MenuItem::Action { id: ID_AUTOSTART, text: "Iniciar com o Windows", checked: autostart },
            MenuItem::Separator,
            MenuItem::Action { id: ID_QUIT, text: "Sair", checked: false },
        ]);

        match win::popup_menu(hwnd, &items) {
            ID_REFRESH => {
                let _ = self.tx.send(Cmd::Refresh);
            }
            ID_USE_LIVE => {
                // Os números antigos podem ser de outra conta.
                self.usage = None;
                self.status = Status::Loading;
                self.warn = None;
                let _ = self.tx.send(Cmd::UseLiveAccount);
            }
            ID_AUTOSTART => {
                if let Err(e) = win::set_autostart(!autostart) {
                    log::error!("iniciar com o Windows: {e}");
                }
            }
            ID_QUIT => {
                self.track_position();
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            _ => {}
        }
    }

    fn next_repaint(&self) -> StdDuration {
        let last_minute = self
            .usage
            .as_ref()
            .and_then(|u| u.resets_at)
            .is_some_and(|r| (r - Utc::now()).num_seconds() < 60);
        if last_minute {
            StdDuration::from_secs(1)
        } else {
            TOPMOST_EVERY
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.drain();
        self.ensure_window(frame);
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(ctx, |ui| self.draw(ui));
        self.enforce_width(ctx);
        self.track_position();
        ctx.request_repaint_after(self.next_repaint());
    }
}
