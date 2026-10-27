//! O login OAuth que o app mantém como seu.
//!
//! O ponto de partida é o login do Claude Code (`~/.claude/.credentials.json`).
//! O app guarda uma cópia própria, criptografada com DPAPI, e continua lendo a
//! mesma conta mesmo que o Claude Code troque de conta ou saia.
//!
//! Renovar um token rotaciona o refresh token. Enquanto a cópia e o Claude
//! Code estão na mesma cadeia de login, renovar só a cópia mataria o login do
//! Claude Code. Por isso, antes de renovar, o app adota um token mais novo que
//! o Claude Code já tenha obtido. Depois de renovar, entrega os tokens novos
//! ao arquivo do Claude Code se ele ainda tiver o refresh token antigo.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::store::{self, AccountInfo};

pub const BETA: &str = "oauth-2025-04-20";
const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const DEFAULT_SCOPES: &str = "user:profile user:inference user:sessions:claude_code \
                              user:mcp_servers user:file_upload user:plugins";
const RENEW_MARGIN_MS: i64 = 5 * 60 * 1000;
/// Uma renovação forçada (depois de um 401) no máximo a cada 30 min, para
/// não girar a cadeia a cada ciclo se o 401 tiver outra causa.
const FORCED_RENEW_GAP: Duration = Duration::from_secs(30 * 60);

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Login {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub refresh_token_expires_at: Option<i64>,
    pub scopes: Vec<String>,
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
    pub organization_uuid: Option<String>,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub imported_at: Option<String>,
}

/// O que o arquivo do Claude Code tem agora.
#[derive(Debug, Clone)]
pub struct Live {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub refresh_token_expires_at: Option<i64>,
    pub scopes: Vec<String>,
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
    pub organization_uuid: Option<String>,
}

#[derive(Debug)]
pub enum AuthError {
    NoLogin,
    Expired,
    Net(String),
}

enum RenewError {
    InvalidGrant,
    InvalidScope,
    Other(String),
}

#[derive(Deserialize)]
struct TokenResp {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
}

/// Os primeiros 16 hex do SHA-256: o único jeito em que um token aparece no log.
pub fn fingerprint(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn num(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))
}

pub fn read_live(path: &Path) -> Option<Live> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let o = v.get("claudeAiOauth")?;
    let s = |k: &str| o.get(k).and_then(Value::as_str).map(str::to_owned);
    Some(Live {
        access_token: s("accessToken")?,
        refresh_token: s("refreshToken")?,
        expires_at: o.get("expiresAt").and_then(num).unwrap_or(0),
        refresh_token_expires_at: o.get("refreshTokenExpiresAt").and_then(num),
        scopes: o
            .get("scopes")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
            .unwrap_or_default(),
        subscription_type: s("subscriptionType"),
        rate_limit_tier: s("rateLimitTier"),
        organization_uuid: v
            .get("organizationUuid")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// Entrega os tokens renovados ao arquivo do Claude Code, só se ele ainda
/// tiver o refresh token que acabou de ser trocado. Mantém todas as outras
/// chaves (`mcpOAuth` etc.), grava sem BOM e relê para conferir.
/// `Ok(false)` significa que o arquivo não era da mesma cadeia e ficou intocado.
pub fn write_back_live(path: &Path, old_refresh: &str, new: &Login) -> Result<bool, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(false);
    };
    let mut v: Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    let Some(o) = v.get_mut("claudeAiOauth").and_then(Value::as_object_mut) else {
        return Ok(false);
    };
    if o.get("refreshToken").and_then(Value::as_str) != Some(old_refresh) {
        return Ok(false);
    }
    o.insert("accessToken".into(), new.access_token.clone().into());
    o.insert("refreshToken".into(), new.refresh_token.clone().into());
    o.insert("expiresAt".into(), new.expires_at.into());
    let out = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    store::write_atomic(path, out.as_bytes()).map_err(|e| e.to_string())?;
    match read_live(path) {
        Some(back) if back.access_token == new.access_token => Ok(true),
        _ => Err("o arquivo relido não confere com o que foi gravado".into()),
    }
}

pub fn plan_label(tier: Option<&str>, sub: Option<&str>) -> Option<String> {
    let t = tier.unwrap_or_default().to_ascii_lowercase();
    let s = sub.unwrap_or_default().to_ascii_lowercase();
    let label = if t.contains("20x") {
        "Max 20x"
    } else if t.contains("5x") {
        "Max 5x"
    } else {
        match s.as_str() {
            "max" => "Max",
            "pro" => "Pro",
            "team" => "Team",
            "enterprise" => "Enterprise",
            "free" => "Free",
            _ => return None,
        }
    };
    Some(label.to_owned())
}

impl Login {
    fn from_live(live: &Live) -> Self {
        let mut l = Login {
            organization_uuid: live.organization_uuid.clone(),
            imported_at: Some(Utc::now().to_rfc3339()),
            ..Default::default()
        };
        l.adopt(live);
        l
    }

    fn adopt(&mut self, live: &Live) {
        self.access_token = live.access_token.clone();
        self.refresh_token = live.refresh_token.clone();
        self.expires_at = live.expires_at;
        if live.refresh_token_expires_at.is_some() {
            self.refresh_token_expires_at = live.refresh_token_expires_at;
        }
        if !live.scopes.is_empty() {
            self.scopes = live.scopes.clone();
        }
        if live.subscription_type.is_some() {
            self.subscription_type = live.subscription_type.clone();
        }
        if live.rate_limit_tier.is_some() {
            self.rate_limit_tier = live.rate_limit_tier.clone();
        }
    }

    /// O login do arquivo é desta conta? Pela org quando os dois lados a
    /// têm. A entrada do próprio Claude Code não grava a org, e nesse caso
    /// vale o mesmo refresh token ou a mesma cadeia (`refreshTokenExpiresAt`
    /// não muda entre renovações).
    fn same_account(&self, live: &Live) -> bool {
        match (&self.organization_uuid, &live.organization_uuid) {
            (Some(a), Some(b)) => a == b,
            _ => {
                live.refresh_token == self.refresh_token
                    || (self.refresh_token_expires_at.is_some()
                        && self.refresh_token_expires_at == live.refresh_token_expires_at)
            }
        }
    }

    fn needs_renewal(&self) -> bool {
        self.expires_at - now_ms() < RENEW_MARGIN_MS
    }
}

pub struct Auth {
    http: Client,
    cred: PathBuf,
    login: Option<Login>,
    forced_at: Option<Instant>,
}

impl Auth {
    pub fn new(http: Client) -> Self {
        let login: Option<Login> = store::load_login();
        if let Some(l) = &login {
            log::info!(
                "login próprio carregado ({}; refresh {})",
                l.email.as_deref().unwrap_or("conta sem email"),
                fingerprint(&l.refresh_token)
            );
        }
        Self {
            http,
            cred: store::cred_path(),
            login,
            forced_at: None,
        }
    }

    pub fn has_login(&self) -> bool {
        self.login.is_some()
    }

    pub fn access_token(&self) -> Option<String> {
        self.login.as_ref().map(|l| l.access_token.clone())
    }

    pub fn account(&self) -> Option<AccountInfo> {
        self.login.as_ref().map(|l| AccountInfo {
            email: l.email.clone(),
            org: l.organization_uuid.clone(),
            plan: plan_label(l.rate_limit_tier.as_deref(), l.subscription_type.as_deref()),
        })
    }

    /// Passa a seguir a conta que está agora no Claude Code.
    pub fn import_live(&mut self) -> Result<(), AuthError> {
        let live = read_live(&self.cred).ok_or(AuthError::NoLogin)?;
        let login = Login::from_live(&live);
        log::info!(
            "importando o login do Claude Code (refresh {})",
            fingerprint(&login.refresh_token)
        );
        store::save_login(&login);
        self.login = Some(login);
        self.ensure_fresh(false)?;
        self.fill_profile();
        if let Some(l) = &self.login {
            store::save_login(l);
        }
        Ok(())
    }

    /// Adota o token do arquivo do Claude Code quando ele é desta conta e é
    /// diferente do nosso. Com `require_newer`, só se ele expirar depois.
    pub fn sync_from_live(&mut self, require_newer: bool) -> bool {
        let Some(live) = read_live(&self.cred) else {
            return false;
        };
        let Some(login) = self.login.as_mut() else {
            return false;
        };
        if live.access_token == login.access_token || !login.same_account(&live) {
            return false;
        }
        if require_newer && live.expires_at <= login.expires_at {
            return false;
        }
        login.adopt(&live);
        store::save_login(&*login);
        log::info!(
            "adotado o token renovado pelo Claude Code (refresh {})",
            fingerprint(&login.refresh_token)
        );
        true
    }

    /// Garante um access token válido por mais alguns minutos. Com `force`,
    /// renova mesmo assim (depois de um 401), no máximo uma vez a cada 30 min.
    pub fn ensure_fresh(&mut self, force: bool) -> Result<(), AuthError> {
        let needs = self.login.as_ref().ok_or(AuthError::NoLogin)?.needs_renewal();
        if force {
            if self.forced_at.is_some_and(|t| t.elapsed() < FORCED_RENEW_GAP) {
                return Err(AuthError::Expired);
            }
            self.forced_at = Some(Instant::now());
        } else if !needs {
            return Ok(());
        }
        // O Claude Code pode já ter renovado este mesmo login.
        if self.sync_from_live(true)
            && !self.login.as_ref().is_some_and(Login::needs_renewal)
        {
            return Ok(());
        }
        self.renew()
    }

    fn renew(&mut self) -> Result<(), AuthError> {
        let old = self.login.clone().ok_or(AuthError::NoLogin)?;
        let scopes = if old.scopes.is_empty() {
            DEFAULT_SCOPES.to_owned()
        } else {
            old.scopes.join(" ")
        };
        let mut resp = self.post_token(&old.refresh_token, &scopes);
        if matches!(resp, Err(RenewError::InvalidScope)) {
            resp = self.post_token(&old.refresh_token, DEFAULT_SCOPES);
        }
        match resp {
            Ok(tok) => {
                let mut new = old.clone();
                new.access_token = tok.access_token;
                if let Some(r) = tok.refresh_token {
                    new.refresh_token = r;
                }
                new.expires_at = now_ms() + tok.expires_in.unwrap_or(3600) * 1000;
                if let Some(s) = tok.scope {
                    let v: Vec<String> = s.split_whitespace().map(str::to_owned).collect();
                    if !v.is_empty() {
                        new.scopes = v;
                    }
                }
                // A cópia primeiro: é a única que guarda o refresh token novo
                // se a entrega ao Claude Code falhar.
                store::save_login(&new);
                log::info!(
                    "login renovado (refresh {} -> {})",
                    fingerprint(&old.refresh_token),
                    fingerprint(&new.refresh_token)
                );
                match write_back_live(&self.cred, &old.refresh_token, &new) {
                    Ok(true) => log::info!("tokens novos entregues ao .credentials.json"),
                    Ok(false) => {}
                    Err(e) => log::warn!("não foi possível atualizar o .credentials.json: {e}"),
                }
                self.login = Some(new);
                Ok(())
            }
            Err(RenewError::InvalidGrant) => {
                log::warn!(
                    "refresh token recusado (refresh {})",
                    fingerprint(&old.refresh_token)
                );
                if self.sync_from_live(false) {
                    Ok(())
                } else {
                    Err(AuthError::Expired)
                }
            }
            Err(RenewError::InvalidScope) => Err(AuthError::Net("escopo recusado".into())),
            Err(RenewError::Other(e)) => Err(AuthError::Net(e)),
        }
    }

    /// Nunca é repetido: se a resposta se perder, o servidor pode já ter
    /// rotacionado o token.
    fn post_token(&self, refresh: &str, scope: &str) -> Result<TokenResp, RenewError> {
        let body = serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh,
            "client_id": CLIENT_ID,
            "scope": scope,
        });
        let resp = self
            .http
            .post(TOKEN_URL)
            .json(&body)
            .send()
            .map_err(|e| RenewError::Other(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp.text().unwrap_or_default();
        if (200..300).contains(&status) {
            return serde_json::from_str(&text)
                .map_err(|e| RenewError::Other(format!("resposta inválida: {e}")));
        }
        let err = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_default();
        match (status, err.as_str()) {
            (400 | 401, "invalid_grant") => Err(RenewError::InvalidGrant),
            (400, "invalid_scope") => Err(RenewError::InvalidScope),
            (c, _) => Err(RenewError::Other(format!("HTTP {c}"))),
        }
    }

    /// Email, org e plano, confirmados pelo servidor.
    fn fill_profile(&mut self) {
        let Some(login) = self.login.as_mut() else {
            return;
        };
        let resp = self
            .http
            .get(PROFILE_URL)
            .bearer_auth(&login.access_token)
            .header("anthropic-beta", BETA)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json::<Value>());
        let v = match resp {
            Ok(v) => v,
            Err(e) => {
                log::warn!("perfil indisponível: {e}");
                return;
            }
        };
        let acc = &v["account"];
        let org = &v["organization"];
        if let Some(e) = acc["email_address"].as_str().or(acc["email"].as_str()) {
            login.email = Some(e.to_owned());
        }
        if let Some(n) = acc["display_name"].as_str().or(acc["full_name"].as_str()) {
            login.display_name = Some(n.to_owned());
        }
        if let Some(u) = org["uuid"].as_str() {
            if login.organization_uuid.as_deref().is_some_and(|o| o != u) {
                log::warn!("a org do arquivo difere da org do servidor; valendo a do servidor");
            }
            login.organization_uuid = Some(u.to_owned());
        }
        if let Some(t) = org["rate_limit_tier"].as_str() {
            login.rate_limit_tier = Some(t.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("usage-bar-rs-{tag}-{}.json", std::process::id()))
    }

    fn login(refresh: &str) -> Login {
        Login {
            access_token: "at-new".into(),
            refresh_token: refresh.into(),
            expires_at: 1_900_000_000_000,
            ..Default::default()
        }
    }

    const LIVE: &str = "\u{feff}{\"claudeAiOauth\":{\"accessToken\":\"at-old\",\"refreshToken\":\"rt-old\",\
        \"expiresAt\":1,\"refreshTokenExpiresAt\":5,\"scopes\":[\"user:inference\"]},\
        \"organizationUuid\":\"org-1\",\"mcpOAuth\":{\"srv\":{\"token\":\"keep\"}}}";

    #[test]
    fn write_back_keeps_other_keys_and_drops_bom() {
        let p = temp_file("wb");
        std::fs::write(&p, LIVE).unwrap();
        let new = login("rt-new");
        assert_eq!(write_back_live(&p, "rt-old", &new), Ok(true));

        let raw = std::fs::read(&p).unwrap();
        assert!(!raw.starts_with(&[0xEF, 0xBB, 0xBF]));
        let v: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["mcpOAuth"]["srv"]["token"], "keep");
        assert_eq!(v["organizationUuid"], "org-1");
        assert_eq!(v["claudeAiOauth"]["refreshToken"], "rt-new");
        assert_eq!(v["claudeAiOauth"]["accessToken"], "at-new");
        assert_eq!(v["claudeAiOauth"]["refreshTokenExpiresAt"], 5);
        assert_eq!(v["claudeAiOauth"]["scopes"][0], "user:inference");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn write_back_leaves_other_chain_alone() {
        let p = temp_file("other");
        std::fs::write(&p, LIVE).unwrap();
        assert_eq!(write_back_live(&p, "rt-different", &login("rt-new")), Ok(false));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), LIVE);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn same_account_rules() {
        let p = temp_file("same");
        std::fs::write(&p, LIVE).unwrap();
        let live = read_live(&p).unwrap();
        let _ = std::fs::remove_file(&p);

        let mut l = login("rt-x");
        l.organization_uuid = Some("org-1".into());
        assert!(l.same_account(&live));
        l.organization_uuid = Some("org-2".into());
        assert!(!l.same_account(&live));
        // Sem org: vale a cadeia.
        l.organization_uuid = None;
        l.refresh_token_expires_at = Some(5);
        assert!(l.same_account(&live));
        l.refresh_token_expires_at = Some(6);
        assert!(!l.same_account(&live));
    }

    #[test]
    fn plan_labels() {
        assert_eq!(plan_label(Some("default_claude_max_20x"), Some("max")).as_deref(), Some("Max 20x"));
        assert_eq!(plan_label(Some("default_claude_max_5x"), None).as_deref(), Some("Max 5x"));
        assert_eq!(plan_label(None, Some("pro")).as_deref(), Some("Pro"));
        assert_eq!(plan_label(None, None), None);
    }
}
