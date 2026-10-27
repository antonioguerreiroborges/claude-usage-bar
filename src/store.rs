//! Arquivos locais: config.json (sem segredos), login.bin (DPAPI) e app.log,
//! todos em %LOCALAPPDATA%\UsageBarRs.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const LOG_CAP: u64 = 256 * 1024;

pub fn data_dir() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
    let dir = base.join("UsageBarRs");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Arquivo de login do Claude Code. `USAGE_BAR_CRED` redireciona em testes.
pub fn cred_path() -> PathBuf {
    if let Some(p) = std::env::var_os("USAGE_BAR_CRED") {
        return PathBuf::from(p);
    }
    dirs::home_dir()
        .unwrap_or_default()
        .join(".claude")
        .join(".credentials.json")
}

fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

fn login_path() -> PathBuf {
    data_dir().join("login.bin")
}

pub fn init_log() {
    let path = data_dir().join("app.log");
    if fs::metadata(&path).map(|m| m.len() > LOG_CAP).unwrap_or(false) {
        let _ = fs::rename(&path, path.with_extension("log.1"));
    }
    if let Ok(f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let cfg = simplelog::ConfigBuilder::new()
            .add_filter_allow_str("usage_bar_rs")
            .build();
        let _ = simplelog::WriteLogger::init(log::LevelFilter::Info, cfg, f);
    }
}

/// Grava em um arquivo temporário ao lado e troca de uma vez, para que um
/// leitor nunca veja o arquivo pela metade.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".usage-bar.tmp");
    let tmp = path.with_file_name(name);
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
pub struct AccountInfo {
    pub email: Option<String>,
    pub org: Option<String>,
    pub plan: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LastUsage {
    pub utilization: f64,
    pub resets_at: Option<DateTime<Utc>>,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// Posição em pixels físicos, as coordenadas dos monitores.
    pub x: Option<i32>,
    pub y: Option<i32>,
    /// Largura em pontos lógicos, a mesma em qualquer escala. O antigo
    /// `width` em pixels é ignorado ao carregar.
    pub width_pt: Option<f32>,
    pub refresh_secs: u64,
    pub account: Option<AccountInfo>,
    pub last: Option<LastUsage>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width_pt: None,
            refresh_secs: 180,
            account: None,
            last: None,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        let Ok(text) = fs::read_to_string(&path) else {
            return Self::default();
        };
        match serde_json::from_str(&text) {
            Ok(cfg) => cfg,
            Err(e) => {
                log::warn!("config.json corrompido ({e}); usando padrões");
                let _ = fs::rename(&path, path.with_extension("json.corrupt"));
                Self::default()
            }
        }
    }

    pub fn save(&self) {
        let r = serde_json::to_string_pretty(self)
            .map_err(|e| e.to_string())
            .and_then(|t| write_atomic(&config_path(), t.as_bytes()).map_err(|e| e.to_string()));
        if let Err(e) = r {
            log::error!("falha ao salvar config.json: {e}");
        }
    }
}

pub fn load_login<T: DeserializeOwned>() -> Option<T> {
    let raw = fs::read(login_path()).ok()?;
    let plain = crate::win::unprotect(&raw)
        .map_err(|e| log::warn!("login.bin ilegível: {e}"))
        .ok()?;
    serde_json::from_slice(&plain)
        .map_err(|e| log::warn!("login.bin inválido: {e}"))
        .ok()
}

pub fn save_login<T: Serialize>(login: &T) {
    let r = serde_json::to_vec(login)
        .map_err(|e| e.to_string())
        .and_then(|plain| crate::win::protect(&plain))
        .and_then(|sealed| write_atomic(&login_path(), &sealed).map_err(|e| e.to_string()));
    if let Err(e) = r {
        log::error!("falha ao salvar login.bin: {e}");
    }
}
