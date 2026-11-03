//! OAuth Device Code Grant Flow for a public client: no client secret is shipped with the app,
//! and the user signs in on twitch.tv in their own browser.
//! https://dev.twitch.tv/docs/authentication/getting-tokens-oauth/#device-code-grant-flow

use std::time::{Duration, Instant};

use reqwest::{Client, Response, StatusCode};
use serde::Deserialize;

use super::store::Credentials;
use super::TwitchError;

const DEVICE_URL: &str = "https://id.twitch.tv/oauth2/device";
const TOKEN_URL: &str = "https://id.twitch.tv/oauth2/token";
const VALIDATE_URL: &str = "https://id.twitch.tv/oauth2/validate";
const REVOKE_URL: &str = "https://id.twitch.tv/oauth2/revoke";

/// Sign-in is only used for badge images, which need no scope; Twitch requires at least one,
/// so we ask for the read-only chat scope.
pub const SCOPES: &str = "user:read:chat";

const CODE_EXPIRED: &str = "Код входа устарел. Нажмите «Войти через Twitch» ещё раз.";

#[derive(Clone, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
}

#[derive(Clone, Deserialize)]
pub struct TokenInfo {
    pub login: String,
}

#[derive(Deserialize)]
struct ErrorBody {
    #[serde(default)]
    message: String,
}

/// Twitch reports OAuth errors as `{"status": 400, "message": "..."}`.
async fn error_message(response: Response) -> String {
    let status = response.status();
    match response.json::<ErrorBody>().await {
        Ok(body) if !body.message.is_empty() => body.message,
        _ => status.to_string(),
    }
}

pub async fn start_device_flow(http: &Client, client_id: &str) -> Result<DeviceCode, TwitchError> {
    let response = http
        .post(DEVICE_URL)
        .form(&[("client_id", client_id), ("scopes", SCOPES)])
        .send()
        .await?;
    if response.status().is_success() {
        return Ok(response.json().await?);
    }
    let message = error_message(response).await;
    Err(TwitchError::Api(match message.to_lowercase().as_str() {
        "invalid client" | "invalid client id" => "Twitch не принял Client ID. Проверьте, что он скопирован \
             полностью и что у приложения на dev.twitch.tv тип клиента «Публичный»."
            .into(),
        _ => format!("Twitch отклонил запрос входа: {message}"),
    }))
}

/// Polls until the user confirms the code on twitch.tv, declines it, or the code expires.
pub async fn wait_for_token(
    http: &Client,
    client_id: &str,
    device: &DeviceCode,
) -> Result<Credentials, TwitchError> {
    let deadline = Instant::now() + Duration::from_secs(device.expires_in);
    let mut interval = device.interval.max(1);

    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        if Instant::now() >= deadline {
            return Err(TwitchError::Api(CODE_EXPIRED.into()));
        }

        let sent = http
            .post(TOKEN_URL)
            .form(&[
                ("client_id", client_id),
                ("scopes", SCOPES),
                ("device_code", &device.device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await;
        // A short network hiccup while the user is in the browser should not cancel the login.
        let Ok(response) = sent else { continue };

        if response.status().is_success() {
            let token: TokenResponse = response.json().await?;
            return Ok(Credentials {
                client_id: client_id.to_owned(),
                access_token: token.access_token,
                refresh_token: token.refresh_token,
            });
        }

        let message = error_message(response).await;
        match message.as_str() {
            "authorization_pending" => {}
            "slow_down" => interval += 5,
            "invalid device code" => return Err(TwitchError::Api(CODE_EXPIRED.into())),
            _ => return Err(TwitchError::Api(format!("Вход не выполнен: {message}"))),
        }
    }
}

/// Twitch requires validating tokens on startup and then hourly.
pub async fn validate(http: &Client, access_token: &str) -> Result<TokenInfo, TwitchError> {
    let response = http
        .get(VALIDATE_URL)
        .header("Authorization", format!("OAuth {access_token}"))
        .send()
        .await?;
    match response.status() {
        s if s.is_success() => Ok(response.json().await?),
        StatusCode::UNAUTHORIZED => Err(TwitchError::Unauthorized),
        _ => Err(TwitchError::Api(format!(
            "Twitch не смог проверить вход: {}",
            error_message(response).await
        ))),
    }
}

/// Public clients refresh without a secret; Twitch rotates the refresh token every time.
pub async fn refresh(http: &Client, credentials: &mut Credentials) -> Result<(), TwitchError> {
    let response = http
        .post(TOKEN_URL)
        .form(&[
            ("client_id", credentials.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", credentials.refresh_token.as_str()),
        ])
        .send()
        .await?;
    match response.status() {
        s if s.is_success() => {
            let token: TokenResponse = response.json().await?;
            credentials.access_token = token.access_token;
            credentials.refresh_token = token.refresh_token;
            Ok(())
        }
        StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED => Err(TwitchError::Unauthorized),
        _ => Err(TwitchError::Api(format!(
            "Twitch не обновил вход: {}",
            error_message(response).await
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hits the real Twitch server: `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn unknown_client_id_gives_readable_error() {
        let err = start_device_flow(&Client::new(), "definitely-not-a-client-id")
            .await
            .err()
            .expect("Twitch must reject an unknown client id");
        assert!(err.to_string().contains("Client ID"), "got: {err}");
    }
}

/// Best effort: signing out must work offline too, so errors are ignored.
pub async fn revoke(http: &Client, credentials: &Credentials) {
    let _ = http
        .post(REVOKE_URL)
        .form(&[
            ("client_id", credentials.client_id.as_str()),
            ("token", credentials.access_token.as_str()),
        ])
        .send()
        .await;
}
