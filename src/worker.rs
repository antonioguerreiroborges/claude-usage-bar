//! Thread de trabalho: dona do login e de toda a rede. Conversa com a UI só
//! por canais, então nada de rede roda na thread da janela.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration as StdDuration;

use chrono::{Duration, Utc};
use eframe::egui;
use reqwest::blocking::Client;

use crate::auth::{Auth, AuthError};
use crate::store::{AccountInfo, LastUsage};
use crate::usage::{self, FiveHour, UsageError};

pub enum Cmd {
    Refresh,
    UseLiveAccount,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Loading,
    Ok,
    NoLogin,
    Expired,
    NoPlan,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub status: Status,
    /// `None` quando a leitura falhou: a UI mantém o último valor.
    pub usage: Option<LastUsage>,
    pub warn: Option<String>,
    pub account: Option<AccountInfo>,
}

enum Fail {
    Auth(AuthError),
    Usage(UsageError),
}

impl From<AuthError> for Fail {
    fn from(e: AuthError) -> Self {
        Fail::Auth(e)
    }
}

impl From<UsageError> for Fail {
    fn from(e: UsageError) -> Self {
        Fail::Usage(e)
    }
}

pub fn spawn(ctx: egui::Context, refresh_secs: u64) -> (Sender<Cmd>, Receiver<Snapshot>) {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (snap_tx, snap_rx) = mpsc::channel();
    thread::Builder::new()
        .name("usage-worker".into())
        .spawn(move || run(ctx, refresh_secs, cmd_rx, snap_tx))
        .expect("não foi possível iniciar a thread de trabalho");
    (cmd_tx, snap_rx)
}

fn run(ctx: egui::Context, refresh_secs: u64, cmds: Receiver<Cmd>, out: Sender<Snapshot>) {
    let http = match Client::builder()
        .timeout(StdDuration::from_secs(20))
        .user_agent(concat!("usage-bar-rs/", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log::error!("cliente HTTP: {e}");
            return;
        }
    };
    let mut auth = Auth::new(http.clone());
    let interval = Duration::seconds(refresh_secs.clamp(10, 3600) as i64);
    // Tudo pelo relógio de parede, para continuar certo depois de suspender.
    let mut next = Utc::now();

    loop {
        if Utc::now() >= next {
            let (snap, wait) = cycle(&http, &mut auth);
            let now = Utc::now();
            next = now + wait.unwrap_or(interval);
            if let Some(r) = snap.usage.as_ref().and_then(|u| u.resets_at) {
                if r > now && r < next {
                    next = r + Duration::seconds(3);
                }
            }
            if out.send(snap).is_err() {
                return;
            }
            ctx.request_repaint();
        }
        let left = (next - Utc::now())
            .to_std()
            .unwrap_or_default()
            .clamp(StdDuration::from_secs(1), StdDuration::from_secs(30));
        match cmds.recv_timeout(left) {
            Ok(Cmd::Refresh) => next = Utc::now(),
            Ok(Cmd::UseLiveAccount) => {
                if let Err(e) = auth.import_live() {
                    log::warn!("importação do login falhou: {e:?}");
                }
                next = Utc::now();
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn cycle(http: &Client, auth: &mut Auth) -> (Snapshot, Option<Duration>) {
    let res = fetch_once(http, auth);
    let mut snap = Snapshot {
        status: Status::Ok,
        usage: None,
        warn: None,
        account: auth.account(),
    };
    let mut wait = None;
    match res {
        Ok(fh) => {
            snap.usage = Some(LastUsage {
                utilization: fh.utilization,
                resets_at: fh.resets_at,
                fetched_at: Utc::now(),
            });
        }
        Err(Fail::Auth(AuthError::NoLogin)) => snap.status = Status::NoLogin,
        Err(Fail::Auth(AuthError::Expired)) | Err(Fail::Usage(UsageError::Unauthorized)) => {
            snap.status = Status::Expired
        }
        Err(Fail::Usage(UsageError::NoPlan)) => snap.status = Status::NoPlan,
        Err(Fail::Usage(UsageError::RateLimited)) => {
            snap.warn = Some("Muitas requisições; nova tentativa em 5 min".into());
            wait = Some(Duration::minutes(5));
        }
        Err(Fail::Auth(AuthError::Net(e))) | Err(Fail::Usage(UsageError::Other(e))) => {
            snap.warn = Some(format!("Falha ao atualizar: {e}"));
        }
    }
    if snap.status != Status::Ok || snap.warn.is_some() {
        log::info!("ciclo: {:?} {:?}", snap.status, snap.warn);
    }
    (snap, wait)
}

fn fetch_once(http: &Client, auth: &mut Auth) -> Result<FiveHour, Fail> {
    if !auth.has_login() {
        auth.import_live()?;
    }
    auth.sync_from_live(true);
    auth.ensure_fresh(false)?;
    let token = auth.access_token().ok_or(AuthError::NoLogin)?;
    match usage::fetch(http, &token) {
        Err(UsageError::Unauthorized) => {
            auth.ensure_fresh(true)?;
            let token = auth.access_token().ok_or(AuthError::NoLogin)?;
            Ok(usage::fetch(http, &token)?)
        }
        r => Ok(r?),
    }
}
