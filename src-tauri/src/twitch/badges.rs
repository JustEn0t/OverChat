//! Badge images from Helix. Twitch only serves them to signed-in apps, so this is the one
//! thing the optional sign-in is for; without it the overlay shows letter badges.

use std::collections::HashMap;

use reqwest::Client;
use serde::Deserialize;

use super::store::Credentials;
use super::TwitchError;

const HELIX: &str = "https://api.twitch.tv/helix";

/// "set_id/version" → (title, image url)
pub type BadgeMap = HashMap<String, (String, String)>;

#[derive(Deserialize)]
struct BadgeSets {
    data: Vec<BadgeSet>,
}

#[derive(Deserialize)]
struct BadgeSet {
    set_id: String,
    versions: Vec<BadgeVersion>,
}

#[derive(Deserialize)]
struct BadgeVersion {
    id: String,
    title: String,
    image_url_2x: String,
}

async fn fetch_sets(http: &Client, credentials: &Credentials, path: &str) -> Result<BadgeSets, TwitchError> {
    Ok(http
        .get(format!("{HELIX}{path}"))
        .bearer_auth(&credentials.access_token)
        .header("Client-Id", &credentials.client_id)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// Global badges first, then the channel's own (custom sub badges override global ones).
pub async fn fetch(http: &Client, credentials: &Credentials, room_id: &str) -> Option<BadgeMap> {
    let mut map = BadgeMap::new();
    let paths = [
        "/chat/badges/global".to_owned(),
        format!("/chat/badges?broadcaster_id={room_id}"),
    ];
    for path in paths {
        if let Ok(sets) = fetch_sets(http, credentials, &path).await {
            for set in sets.data {
                for version in set.versions {
                    map.insert(
                        format!("{}/{}", set.set_id, version.id),
                        (version.title, version.image_url_2x),
                    );
                }
            }
        }
    }
    (!map.is_empty()).then_some(map)
}
