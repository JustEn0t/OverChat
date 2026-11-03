//! The message shape the overlay renders. Mirrors `ChatMessage` in src/shared/types.ts.

use serde::Serialize;

/// Twitch's palette for users who never picked a name color.
const DEFAULT_COLORS: [&str; 15] = [
    "#FF0000", "#0000FF", "#008000", "#B22222", "#FF7F50", "#9ACD32", "#FF4500", "#2E8B57",
    "#DAA520", "#D2691E", "#5F9EA0", "#1E90FF", "#FF69B4", "#8A2BE2", "#00FF7F",
];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    /// Twitch user id: lets the overlay remove a user's messages after a ban or timeout.
    pub user_id: String,
    pub user: String,
    pub color: String,
    /// Plain text, kept for filters; the overlay renders `fragments`.
    pub text: String,
    /// `/me` message: Twitch shows these in the user's color.
    pub action: bool,
    pub fragments: Vec<Fragment>,
    pub badges: Vec<Badge>,
    pub timestamp: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Fragment {
    Text { text: String },
    Emote { name: String, url: String, zero_width: bool },
}

/// Moderation in the channel: the overlay hides what Twitch chat hides.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ChatDeletion {
    /// A moderator deleted one message.
    Message { id: String },
    /// A user was banned or timed out.
    User { user_id: String },
    /// `/clear`
    All,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Badge {
    pub kind: String,
    pub title: String,
    pub image_url: Option<String>,
}

pub fn default_color(login: &str) -> String {
    let sum: usize = login.bytes().map(usize::from).sum();
    DEFAULT_COLORS[sum % DEFAULT_COLORS.len()].to_owned()
}

pub fn fallback_badge_title(set_id: &str) -> &str {
    match set_id {
        "broadcaster" => "Стример",
        "moderator" | "lead_moderator" => "Модератор",
        "vip" => "VIP",
        "subscriber" | "founder" => "Подписчик",
        other => other,
    }
}
