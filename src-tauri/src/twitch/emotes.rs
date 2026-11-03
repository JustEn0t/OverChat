//! Emotes: Twitch's own (positions come in the IRC `emotes` tag) and 7TV (matched by word).
//! 7TV API: https://7tv.io/v3 — public, no sign-in needed.

use std::collections::HashMap;

use reqwest::Client;
use serde::Deserialize;

use super::message::Fragment;

const SEVENTV_API: &str = "https://7tv.io/v3";
/// 7TV marks emotes drawn on top of the previous one (PETPET, RainTime…) with these flags.
const ACTIVE_ZERO_WIDTH: u32 = 1;
const DATA_ZERO_WIDTH: u32 = 1 << 8;

pub struct Emote {
    pub url: String,
    pub zero_width: bool,
}

/// Emote name → image. Case-sensitive, like in Twitch chat.
pub type EmoteMap = HashMap<String, Emote>;

#[derive(Deserialize)]
struct SevenTvSet {
    #[serde(default)]
    emotes: Vec<SevenTvEmote>,
}

#[derive(Deserialize)]
struct SevenTvEmote {
    name: String,
    #[serde(default)]
    flags: u32,
    data: SevenTvData,
}

#[derive(Deserialize)]
struct SevenTvData {
    #[serde(default)]
    flags: u32,
    host: SevenTvHost,
}

#[derive(Deserialize)]
struct SevenTvHost {
    /// Protocol-relative, e.g. `//cdn.7tv.app/emote/01F…`
    url: String,
}

#[derive(Deserialize)]
struct SevenTvUser {
    emote_set: Option<SevenTvSet>,
}

pub fn twitch_emote_url(id: &str) -> String {
    format!("https://static-cdn.jtvnw.net/emoticons/v2/{id}/default/dark/2.0")
}

fn add_set(map: &mut EmoteMap, set: SevenTvSet) {
    for emote in set.emotes {
        map.insert(
            emote.name,
            Emote {
                url: format!("https:{}/2x.webp", emote.data.host.url),
                zero_width: emote.flags & ACTIVE_ZERO_WIDTH != 0
                    || emote.data.flags & DATA_ZERO_WIDTH != 0,
            },
        );
    }
}

/// Global 7TV emotes plus the channel's set (channel names win on conflicts).
/// Emotes are cosmetic: any failure (no 7TV account, 7TV down) just means fewer emotes.
pub async fn load_7tv(http: &Client, room_id: &str) -> EmoteMap {
    let mut map = EmoteMap::new();

    if let Ok(response) = http.get(format!("{SEVENTV_API}/emote-sets/global")).send().await {
        if let Ok(set) = response.json::<SevenTvSet>().await {
            add_set(&mut map, set);
        }
    }
    if let Ok(response) = http.get(format!("{SEVENTV_API}/users/twitch/{room_id}")).send().await {
        if response.status().is_success() {
            if let Ok(SevenTvUser { emote_set: Some(set) }) = response.json().await {
                add_set(&mut map, set);
            }
        }
    }
    map
}

/// Parses the IRC `emotes` tag: `25:0-4,12-16/1902:6-10` → (start, end, id), sorted.
/// Positions are inclusive and count Unicode characters, not bytes.
fn twitch_emote_ranges(tag: &str) -> Vec<(usize, usize, &str)> {
    let mut ranges = Vec::new();
    for entry in tag.split('/').filter(|e| !e.is_empty()) {
        let Some((id, positions)) = entry.split_once(':') else { continue };
        for position in positions.split(',') {
            if let Some((start, end)) = position.split_once('-') {
                if let (Ok(start), Ok(end)) = (start.parse(), end.parse()) {
                    ranges.push((start, end, id));
                }
            }
        }
    }
    ranges.sort_by_key(|r| r.0);
    ranges
}

fn push_text(fragments: &mut Vec<Fragment>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(Fragment::Text { text: last }) = fragments.last_mut() {
        last.push_str(text);
    } else {
        fragments.push(Fragment::Text { text: text.to_owned() });
    }
}

/// Splits plain text into words and swaps known 7TV names for emotes.
fn push_with_7tv(fragments: &mut Vec<Fragment>, text: &str, seventv: &EmoteMap) {
    for (i, word) in text.split(' ').enumerate() {
        if i > 0 {
            push_text(fragments, " ");
        }
        match seventv.get(word) {
            Some(emote) => {
                // A zero-width emote sits on the previous emote, so the space between them goes.
                if emote.zero_width {
                    if let [.., Fragment::Emote { .. }, Fragment::Text { text }] = fragments.as_slice() {
                        if text == " " {
                            fragments.pop();
                        }
                    }
                }
                fragments.push(Fragment::Emote {
                    name: word.to_owned(),
                    url: emote.url.clone(),
                    zero_width: emote.zero_width,
                });
            }
            None => push_text(fragments, word),
        }
    }
}

pub fn build_fragments(text: &str, emotes_tag: &str, seventv: &EmoteMap) -> Vec<Fragment> {
    let chars: Vec<char> = text.chars().collect();
    let mut fragments = Vec::new();
    let mut cursor = 0;

    for (start, end, id) in twitch_emote_ranges(emotes_tag) {
        if start < cursor || end >= chars.len() || start > end {
            continue; // Overlapping or out-of-range data: keep it as text.
        }
        let before: String = chars[cursor..start].iter().collect();
        push_with_7tv(&mut fragments, &before, seventv);
        fragments.push(Fragment::Emote {
            name: chars[start..=end].iter().collect(),
            url: twitch_emote_url(id),
            zero_width: false,
        });
        cursor = end + 1;
    }
    let rest: String = chars[cursor..].iter().collect();
    push_with_7tv(&mut fragments, &rest, seventv);
    fragments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seventv() -> EmoteMap {
        let mut map = EmoteMap::new();
        map.insert("catJAM".into(), Emote { url: "https://7tv/cat".into(), zero_width: false });
        map.insert("RainTime".into(), Emote { url: "https://7tv/rain".into(), zero_width: true });
        map
    }

    fn emote(name: &str, url: &str, zero_width: bool) -> Fragment {
        Fragment::Emote { name: name.into(), url: url.into(), zero_width }
    }

    fn text(t: &str) -> Fragment {
        Fragment::Text { text: t.into() }
    }

    #[test]
    fn mixes_twitch_and_7tv_emotes() {
        // "Kappa" at 0-4 is a Twitch emote; catJAM comes from 7TV.
        let fragments = build_fragments("Kappa привет catJAM !", "25:0-4", &seventv());
        assert_eq!(
            fragments,
            vec![
                emote("Kappa", &twitch_emote_url("25"), false),
                text(" привет "),
                emote("catJAM", "https://7tv/cat", false),
                text(" !"),
            ]
        );
    }

    #[test]
    fn twitch_positions_count_characters_not_bytes() {
        let fragments = build_fragments("ыы Kappa", "25:3-7", &EmoteMap::new());
        assert_eq!(fragments, vec![text("ыы "), emote("Kappa", &twitch_emote_url("25"), false)]);
    }

    #[test]
    fn zero_width_emote_drops_the_space_before_it() {
        let fragments = build_fragments("catJAM RainTime ok", "", &seventv());
        assert_eq!(
            fragments,
            vec![
                emote("catJAM", "https://7tv/cat", false),
                emote("RainTime", "https://7tv/rain", true),
                text(" ok"),
            ]
        );
    }

    #[test]
    fn bad_emote_tag_falls_back_to_text() {
        let fragments = build_fragments("hi", "25:0-40", &EmoteMap::new());
        assert_eq!(fragments, vec![text("hi")]);
    }

    /// Hits the real 7TV API: `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn loads_7tv_global_and_channel_emotes() {
        // 71092938 is a large channel with a 7TV emote set.
        let map = load_7tv(&Client::new(), "71092938").await;
        assert!(map.len() > 45, "got {} emotes", map.len());
        assert!(map.values().any(|e| e.zero_width));
    }
}
