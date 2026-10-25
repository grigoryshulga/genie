//! The GitHub App a deployment is registered as: its signed identity, the tokens it mints and the
//! calls that verify and list what a person installed.
//!
//! - The **app JWT** (RS256, ten minutes at most) proves the server is the app. It mints and
//!   inspects installation tokens and is never stored.
//! - An **installation token** (an hour) acts as the app on the repositories it is narrowed to.
//!   It does all the background work (fetch, push, requests, checks) and is kept in memory only
//!   ([`super::tokens`]).
//! - A **user token** proves who installed the app and which installations that person can see.
//!   It is used once during connecting and then dropped.
//!
//! Documented in docs/development/git-provider-connection-research.md.

use std::time::Duration;

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use reqwest::Method;
use serde::Serialize;
use serde_json::{Value, json};

use super::hosts::Host;
use super::provider::{Api, ApiError, ApiResult, s};
use crate::config::GithubApp;

/// An installation token and when to stop using it.
pub struct Minted {
    pub token: String,
    pub expires: std::time::SystemTime,
}

/// The decoded body of a PEM block (PKCS#1 `RSA PRIVATE KEY` or PKCS#8 `PRIVATE KEY`) and its kind.
fn pem_der(pem: &str) -> Result<(Vec<u8>, bool), String> {
    let pkcs8 = pem.contains("BEGIN PRIVATE KEY");
    if !pkcs8 && !pem.contains("BEGIN RSA PRIVATE KEY") {
        return Err("the private key is not an RSA PEM (BEGIN RSA PRIVATE KEY or BEGIN PRIVATE KEY)".into());
    }
    let body: String = pem.lines().filter(|l| !l.trim_start().starts_with("-----")).map(str::trim).collect();
    let der = STANDARD.decode(body.as_bytes()).map_err(|_| "the private key PEM is not valid base64".to_string())?;
    Ok((der, pkcs8))
}

/// The app JWT, valid from a minute ago (clock skew) for nine minutes.
pub fn app_jwt(cfg: &GithubApp, now_unix: i64) -> Result<String, String> {
    let pem =
        std::fs::read_to_string(&cfg.private_key_file).map_err(|e| format!("the GitHub App private key {}: {e}", cfg.private_key_file))?;
    sign_jwt(cfg.app_id, &pem, now_unix)
}

fn sign_jwt(app_id: i64, pem: &str, now_unix: i64) -> Result<String, String> {
    let (der, pkcs8) = pem_der(pem)?;
    let key = if pkcs8 { RsaKeyPair::from_pkcs8(&der) } else { RsaKeyPair::from_der(&der) }
        .map_err(|_| "the GitHub App private key cannot be used (is it an RSA key of at least 2048 bits?)".to_string())?;
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let claims = URL_SAFE_NO_PAD.encode(json!({ "iat": now_unix - 60, "exp": now_unix + 540, "iss": app_id }).to_string());
    let signing_input = format!("{header}.{claims}");
    let mut sig = vec![0u8; key.public_modulus_len()];
    key.sign(&RSA_PKCS1_SHA256, &SystemRandom::new(), signing_input.as_bytes(), &mut sig)
        .map_err(|_| "signing the GitHub App token failed".to_string())?;
    Ok(format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(sig)))
}

fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or_default()
}

/// A client of the host's API acting as `token` (an app JWT, an installation token or a user token).
fn acting_as(host: &Host, token: String) -> ApiResult<Api> {
    let mut h = host.clone();
    h.token = Some(token);
    Api::new(&h)
}

fn as_app(host: &Host, cfg: &GithubApp) -> ApiResult<Api> {
    acting_as(host, app_jwt(cfg, unix_now()).map_err(ApiError::Auth)?)
}

/// Mint a token for one installation, narrowed to the repositories `repo_ids`.
pub async fn mint(host: &Host, cfg: &GithubApp, installation: i64, repo_ids: &[i64]) -> ApiResult<Minted> {
    let api = as_app(host, cfg)?;
    let body = json!({ "repository_ids": repo_ids });
    let res = api.send(Method::POST, &format!("/app/installations/{installation}/access_tokens"), &[], Some(body)).await?.body;
    let token = s(&res, "token");
    if token.is_empty() {
        return Err(ApiError::Rejected("GitHub returned no installation token".into()));
    }
    // Stop using it five minutes early, so an operation never starts with a token about to lapse.
    let expires = chrono::DateTime::parse_from_rfc3339(&s(&res, "expires_at"))
        .ok()
        .map(|t| std::time::UNIX_EPOCH + Duration::from_secs(t.timestamp().max(0) as u64))
        .unwrap_or_else(|| std::time::SystemTime::now() + Duration::from_secs(3600))
        .checked_sub(Duration::from_secs(300))
        .unwrap_or(std::time::UNIX_EPOCH);
    Ok(Minted { token, expires })
}

/// What the provider says about an installation (app JWT): suspension, selection, permissions.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installation {
    pub id: i64,
    pub account_login: String,
    pub account_id: i64,
    pub account_type: String,
    /// `all` or `selected`.
    pub selection: String,
    pub permissions: Value,
    pub suspended: bool,
}

fn installation_of(v: &Value) -> Installation {
    Installation {
        id: v["id"].as_i64().unwrap_or_default(),
        account_login: s(&v["account"], "login"),
        account_id: v["account"]["id"].as_i64().unwrap_or_default(),
        account_type: s(&v["account"], "type"),
        selection: if s(v, "repository_selection") == "all" { "all".into() } else { "selected".into() },
        permissions: v["permissions"].clone(),
        suspended: v["suspended_at"].as_str().is_some_and(|t| !t.is_empty()),
    }
}

pub async fn installation(host: &Host, cfg: &GithubApp, id: i64) -> ApiResult<Installation> {
    let api = as_app(host, cfg)?;
    Ok(installation_of(&api.send(Method::GET, &format!("/app/installations/{id}"), &[], None).await?.body))
}

/// Exchange the authorization code of the OAuth leg for the person's user token.
pub async fn exchange_code(
    host: &Host,
    cfg: &GithubApp,
    client_secret: &str,
    code: &str,
    verifier: &str,
    redirect: &str,
) -> ApiResult<String> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("genie")
        .build()
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    let res = http
        .post(format!("{}/login/oauth/access_token", host.url))
        .header("Accept", "application/json")
        .form(&[
            ("client_id", cfg.client_id.as_str()),
            ("client_secret", client_secret),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect),
        ])
        .send()
        .await
        .map_err(|e| ApiError::Unavailable(e.without_url().to_string()))?;
    let body: Value = res.json().await.map_err(|_| ApiError::Rejected("GitHub's answer to the code exchange was not JSON".into()))?;
    match body["access_token"].as_str().filter(|t| !t.is_empty()) {
        Some(t) => Ok(t.to_string()),
        None => Err(ApiError::Auth(format!("GitHub refused the code: {}", s(&body, "error")))),
    }
}

/// The person a user token belongs to.
pub async fn whoami(host: &Host, user_token: &str) -> ApiResult<(i64, String)> {
    let api = acting_as(host, user_token.to_string())?;
    let me = api.send(Method::GET, "/user", &[], None).await?.body;
    Ok((me["id"].as_i64().unwrap_or_default(), s(&me, "login")))
}

/// The installations of this app the person can access (user token).
pub async fn user_installations(host: &Host, user_token: &str) -> ApiResult<Vec<Installation>> {
    let api = acting_as(host, user_token.to_string())?;
    let mut out = Vec::new();
    for page in 1..=10 {
        let body =
            api.send(Method::GET, "/user/installations", &[("per_page", "100".into()), ("page", page.to_string())], None).await?.body;
        let list = body["installations"].as_array().cloned().unwrap_or_default();
        let n = list.len();
        out.extend(list.iter().map(installation_of));
        if n < 100 {
            break;
        }
    }
    Ok(out)
}

/// A repository the installation reaches.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRepo {
    pub id: i64,
    pub full_name: String,
    pub default_branch: String,
    pub private: bool,
    pub can_push: bool,
}

/// The repositories of one installation that the person and the app both reach (user token).
pub async fn user_installation_repos(host: &Host, user_token: &str, installation: i64) -> ApiResult<Vec<ProviderRepo>> {
    let api = acting_as(host, user_token.to_string())?;
    let mut out = Vec::new();
    for page in 1..=20 {
        let body = api
            .send(
                Method::GET,
                &format!("/user/installations/{installation}/repositories"),
                &[("per_page", "100".into()), ("page", page.to_string())],
                None,
            )
            .await?
            .body;
        let list = body["repositories"].as_array().cloned().unwrap_or_default();
        let n = list.len();
        out.extend(list.iter().map(|r| ProviderRepo {
            id: r["id"].as_i64().unwrap_or_default(),
            full_name: s(r, "full_name"),
            default_branch: s(r, "default_branch"),
            private: r["private"].as_bool().unwrap_or(false),
            can_push: r["permissions"]["push"].as_bool().unwrap_or(true),
        }));
        if n < 100 {
            break;
        }
    }
    Ok(out)
}

/// One repository by its provider id, as the installation token sees it (the grant's current name and branch).
pub async fn repo_by_id(host: &Host, token: &str, id: i64) -> ApiResult<ProviderRepo> {
    let api = acting_as(host, token.to_string())?;
    let r = api.send(Method::GET, &format!("/repositories/{id}"), &[], None).await?.body;
    Ok(ProviderRepo {
        id: r["id"].as_i64().unwrap_or(id),
        full_name: s(&r, "full_name"),
        default_branch: s(&r, "default_branch"),
        private: r["private"].as_bool().unwrap_or(false),
        can_push: r["permissions"]["push"].as_bool().unwrap_or(true),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::signature::{KeyPair, RSA_PKCS1_2048_8192_SHA256, UnparsedPublicKey};

    /// A throwaway 2048-bit key as a PEM, in the form GitHub hands out (PKCS#1) or as PKCS#8.
    fn test_key(pkcs1: bool) -> String {
        use aws_lc_rs::encoding::AsDer;
        let key = RsaKeyPair::generate(aws_lc_rs::rsa::KeySize::Rsa2048).unwrap();
        let pkcs8 = AsDer::<aws_lc_rs::encoding::Pkcs8V1Der>::as_der(&key).unwrap();
        let mut der = pkcs8.as_ref().to_vec();
        if pkcs1 {
            // PrivateKeyInfo { version, rsaEncryption algorithm, OCTET STRING { RSAPrivateKey } }: the PKCS#1
            // key is what follows the 26-byte header of a 2048-bit key (long-form lengths).
            assert_eq!(&der[22..24], &[0x04, 0x82]);
            der = der[26..].to_vec();
        }
        let b64 = STANDARD.encode(der);
        let lines: Vec<String> = b64.as_bytes().chunks(64).map(|c| String::from_utf8_lossy(c).into_owned()).collect();
        let label = if pkcs1 { "RSA PRIVATE KEY" } else { "PRIVATE KEY" };
        format!("-----BEGIN {label}-----\n{}\n-----END {label}-----\n", lines.join("\n"))
    }

    #[test]
    fn the_app_jwt_is_rs256_with_the_app_as_issuer_and_verifies_against_the_public_key() {
        for pkcs1 in [true, false] {
            check_jwt(&test_key(pkcs1), pkcs1);
        }
    }

    fn check_jwt(pem: &str, pkcs1: bool) {
        let jwt = sign_jwt(4242, pem, 1_000_000).unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let header: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
        let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(header, json!({"alg": "RS256", "typ": "JWT"}));
        assert_eq!(claims, json!({"iat": 999_940, "exp": 1_000_540, "iss": 4242}));

        let (der, pkcs8) = pem_der(pem).unwrap();
        assert_eq!(pkcs8, !pkcs1);
        let public = if pkcs8 { RsaKeyPair::from_pkcs8(&der) } else { RsaKeyPair::from_der(&der) }.unwrap();
        let public = UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, public.public_key().as_ref().to_vec());
        public.verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &URL_SAFE_NO_PAD.decode(parts[2]).unwrap()).unwrap();
    }

    #[test]
    fn a_key_that_is_not_an_rsa_pem_is_refused_without_echoing_it() {
        let err = sign_jwt(1, "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----", 0).unwrap_err();
        assert!(err.contains("not an RSA PEM") && !err.contains("AAAA"), "{err}");
        let err = sign_jwt(1, "-----BEGIN PRIVATE KEY-----\n!!!\n-----END PRIVATE KEY-----", 0).unwrap_err();
        assert!(err.contains("base64"), "{err}");
    }
}
