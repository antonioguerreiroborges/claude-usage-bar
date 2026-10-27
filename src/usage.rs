//! Leitura do uso da janela de 5 horas.

use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use serde_json::Value;

use crate::auth::BETA;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

#[derive(Debug, Clone, PartialEq)]
pub struct FiveHour {
    pub utilization: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub enum UsageError {
    Unauthorized,
    RateLimited,
    NoPlan,
    Other(String),
}

pub fn fetch(http: &Client, token: &str) -> Result<FiveHour, UsageError> {
    let resp = http
        .get(USAGE_URL)
        .bearer_auth(token)
        .header("anthropic-beta", BETA)
        .send()
        .map_err(|e| UsageError::Other(e.to_string()))?;
    match resp.status().as_u16() {
        401 | 403 => return Err(UsageError::Unauthorized),
        429 => return Err(UsageError::RateLimited),
        c if c >= 400 => return Err(UsageError::Other(format!("HTTP {c}"))),
        _ => {}
    }
    let text = resp.text().map_err(|e| UsageError::Other(e.to_string()))?;
    parse(&text)
}

pub fn parse(text: &str) -> Result<FiveHour, UsageError> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| UsageError::Other(format!("resposta inválida: {e}")))?;
    let five = &v["five_hour"];
    if five.is_null() {
        // A única forma que o servidor tem de dizer que não há plano.
        let no_limits = v["limits"].as_array().is_some_and(|a| a.is_empty());
        if no_limits && v["seven_day"].is_null() {
            return Err(UsageError::NoPlan);
        }
        return Ok(FiveHour {
            utilization: 0.0,
            resets_at: None,
        });
    }
    let utilization = five["utilization"].as_f64().unwrap_or(0.0);
    let resets_at = five["resets_at"]
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));
    Ok(FiveHour {
        utilization,
        resets_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_five_hour_only() {
        let body = r#"{
            "five_hour": {"utilization": 62.0, "resets_at": "2026-10-27T19:00:00.442913+00:00"},
            "seven_day": {"utilization": 30.0, "resets_at": "2026-10-31T10:00:00+00:00"},
            "seven_day_opus": null
        }"#;
        let u = parse(body).unwrap();
        assert_eq!(u.utilization, 62.0);
        assert_eq!(
            u.resets_at.unwrap().to_rfc3339(),
            "2026-10-27T19:00:00.442913+00:00"
        );
    }

    #[test]
    fn idle_session_has_no_reset() {
        let body = r#"{"five_hour": {"utilization": 0.0, "resets_at": null}, "seven_day": null}"#;
        let u = parse(body).unwrap();
        assert_eq!(u.utilization, 0.0);
        assert!(u.resets_at.is_none());
    }

    #[test]
    fn no_plan_shape() {
        let body = r#"{"five_hour": null, "seven_day": null, "limits": []}"#;
        assert!(matches!(parse(body), Err(UsageError::NoPlan)));
    }

    #[test]
    fn missing_five_hour_is_not_no_plan() {
        let body = r#"{"five_hour": null, "seven_day": {"utilization": 3.0}}"#;
        assert_eq!(parse(body).unwrap().utilization, 0.0);
    }
}
