//! Reads a channel's chat anonymously over Twitch IRC (WebSocket): no sign-in, any public channel.
//! https://dev.twitch.tv/docs/chat/irc/

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use tauri::{AppHandle, Emitter};
use tokio::net::TcpStream;
use tokio::time::{interval_at, timeout, Instant};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use super::badges::BadgeMap;
use super::emotes::{self, EmoteMap};
use super::message::{default_color, fallback_badge_title, Badge, ChatDeletion, ChatMessage};
use super::{load_badges, set_chat_status, ChatStatus, TwitchError, CHAT_DELETE_EVENT, CHAT_EVENT};

const IRC_URL: &str = "wss://irc-ws.chat.twitch.tv:443";
const JOIN_TIMEOUT: Duration = Duration::from_secs(10);
/// We ping every 2 minutes, so 4 minutes of silence means the connection is dead.
const PING_EVERY: Duration = Duration::from_secs(120);
const READ_TIMEOUT: Duration = Duration::from_secs(240);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug)]
pub struct IrcLine<'a> {
    tags: HashMap<&'a str, String>,
    prefix: Option<&'a str>,
    command: &'a str,
    params: Vec<&'a str>,
}

impl IrcLine<'_> {
    fn tag(&self, key: &str) -> &str {
        self.tags.get(key).map(String::as_str).unwrap_or("")
    }

    fn trailing(&self) -> &str {
        self.params.last().copied().unwrap_or("")
    }
}

fn unescape_tag(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some(':') => out.push(';'),
            Some('\\') => out.push('\\'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// `@tags :prefix COMMAND param param :trailing`
pub fn parse(line: &str) -> Option<IrcLine<'_>> {
    let mut rest = line.trim_end_matches(['\r', '\n']);
    let mut tags = HashMap::new();

    if let Some(stripped) = rest.strip_prefix('@') {
        let (raw, after) = stripped.split_once(' ')?;
        for pair in raw.split(';') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            tags.insert(key, unescape_tag(value));
        }
        rest = after;
    }

    let mut prefix = None;
    if let Some(stripped) = rest.strip_prefix(':') {
        let (p, after) = stripped.split_once(' ')?;
        prefix = Some(p);
        rest = after;
    }

    let (head, trailing) = match rest.split_once(" :") {
        Some((head, trailing)) => (head, Some(trailing)),
        None => (rest, None),
    };
    let mut words = head.split(' ').filter(|w| !w.is_empty());
    let command = words.next()?;
    let mut params: Vec<&str> = words.collect();
    params.extend(trailing);

    Some(IrcLine { tags, prefix, command, params })
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn to_chat_message(line: &IrcLine, seventv: &EmoteMap, badges: Option<&BadgeMap>) -> ChatMessage {
    let login = line
        .prefix
        .and_then(|p| p.split('!').next())
        .unwrap_or_default();
    let user = match line.tag("display-name") {
        "" => login.to_owned(),
        name => name.to_owned(),
    };
    let color = match line.tag("color") {
        "" => default_color(login),
        color => color.to_owned(),
    };

    let mut text = line.trailing();
    let action = text.starts_with("\u{1}ACTION ") && text.ends_with('\u{1}');
    if action {
        text = &text[8..text.len() - 1];
    }

    let badges = line
        .tag("badges")
        .split(',')
        .filter_map(|b| b.split_once('/'))
        .map(|(set_id, version)| {
            let known = badges.and_then(|map| map.get(&format!("{set_id}/{version}")));
            Badge {
                kind: set_id.to_owned(),
                title: known
                    .map(|(title, _)| title.clone())
                    .unwrap_or_else(|| fallback_badge_title(set_id).to_owned()),
                image_url: known.map(|(_, url)| url.clone()),
            }
        })
        .collect();

    let id = match line.tag("id") {
        "" => format!("irc-{}", now_millis()),
        id => id.to_owned(),
    };

    ChatMessage {
        id,
        user_id: line.tag("user-id").to_owned(),
        user,
        color,
        text: text.to_owned(),
        action,
        fragments: emotes::build_fragments(text, line.tag("emotes"), seventv),
        badges,
        timestamp: now_millis(),
    }
}

/// CLEARMSG / CLEARCHAT → what the overlay should remove.
pub fn to_deletion(line: &IrcLine) -> Option<ChatDeletion> {
    match line.command {
        "CLEARMSG" => match line.tag("target-msg-id") {
            "" => None,
            id => Some(ChatDeletion::Message { id: id.to_owned() }),
        },
        "CLEARCHAT" => Some(match line.tag("target-user-id") {
            "" => ChatDeletion::All,
            user_id => ChatDeletion::User { user_id: user_id.to_owned() },
        }),
        _ => None,
    }
}

async fn send(socket: &mut Socket, line: String) -> Result<(), TwitchError> {
    socket.send(Message::Text(line.into())).await?;
    Ok(())
}

fn suspended(line: &IrcLine, channel: &str) -> Option<TwitchError> {
    (line.command == "NOTICE" && line.tag("msg-id") == "msg_channel_suspended").then(|| {
        TwitchError::Fatal(format!("Канал «{channel}» заблокирован или удалён на Twitch."))
    })
}

/// Joins the channel and returns its Twitch user id (from ROOMSTATE).
async fn join(socket: &mut Socket, channel: &str) -> Result<String, TwitchError> {
    // `justinfanNNNN` is Twitch's read-only anonymous login.
    let nick = format!("justinfan{}", 10_000 + now_millis() % 80_000);
    send(socket, "CAP REQ :twitch.tv/tags twitch.tv/commands".into()).await?;
    send(socket, "PASS SCHMOOPIIE".into()).await?;
    send(socket, format!("NICK {nick}")).await?;
    send(socket, format!("JOIN #{channel}")).await?;

    let deadline = Instant::now() + JOIN_TIMEOUT;
    loop {
        let next = tokio::time::timeout_at(deadline, socket.next()).await.map_err(|_| {
            TwitchError::Fatal(format!(
                "Канал «{channel}» не найден. Проверьте ник — он такой же, как в ссылке twitch.tv/…"
            ))
        })?;
        let Some(message) = next else {
            return Err(TwitchError::Api("Twitch закрыл соединение с чатом.".into()));
        };
        let Message::Text(frame) = message? else { continue };
        for raw in frame.as_str().split("\r\n") {
            let Some(line) = parse(raw) else { continue };
            if let Some(err) = suspended(&line, channel) {
                return Err(err);
            }
            match line.command {
                "PING" => send(socket, format!("PONG :{}", line.trailing())).await?,
                "ROOMSTATE" if !line.tag("room-id").is_empty() => {
                    return Ok(line.tag("room-id").to_owned())
                }
                _ => {}
            }
        }
    }
}

/// Runs one connection until it drops; the caller decides whether to reconnect.
pub async fn run(app: &AppHandle, http: &Client, channel: &str, backoff: &mut u64) -> Result<(), TwitchError> {
    let (mut socket, _) = connect_async(IRC_URL).await?;
    let room_id = join(&mut socket, channel).await?;

    let (seventv, badges) = tokio::join!(emotes::load_7tv(http, &room_id), load_badges(app, http, &room_id));

    *backoff = 1;
    set_chat_status(
        app,
        ChatStatus::Connected {
            channel: channel.to_owned(),
            seventv_emotes: seventv.len(),
            badge_images: badges.is_some(),
        },
    );

    let mut ping = interval_at(Instant::now() + PING_EVERY, PING_EVERY);
    loop {
        tokio::select! {
            next = timeout(READ_TIMEOUT, socket.next()) => {
                let message = match next {
                    Err(_) => return Err(TwitchError::Api("Twitch перестал отвечать.".into())),
                    Ok(None) => return Err(TwitchError::Api("Twitch закрыл соединение.".into())),
                    Ok(Some(message)) => message?,
                };
                let Message::Text(frame) = message else { continue };
                for raw in frame.as_str().split("\r\n") {
                    let Some(line) = parse(raw) else { continue };
                    if let Some(err) = suspended(&line, channel) {
                        return Err(err);
                    }
                    match line.command {
                        "PRIVMSG" => {
                            let _ = app.emit(CHAT_EVENT, to_chat_message(&line, &seventv, badges.as_ref()));
                        }
                        "CLEARMSG" | "CLEARCHAT" => {
                            if let Some(deletion) = to_deletion(&line) {
                                let _ = app.emit(CHAT_DELETE_EVENT, deletion);
                            }
                        }
                        "PING" => send(&mut socket, format!("PONG :{}", line.trailing())).await?,
                        "RECONNECT" => {
                            return Err(TwitchError::Api("Twitch перезапускает сервер чата.".into()))
                        }
                        _ => {}
                    }
                }
            }
            _ = ping.tick() => send(&mut socket, "PING :overchat".into()).await?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twitch::emotes::Emote;
    use crate::twitch::message::Fragment;

    const PRIVMSG: &str = "@badge-info=subscriber/14;badges=moderator/1,subscriber/12;color=;\
display-name=Катя;emotes=25:0-4;id=abc-123;room-id=1;user-id=2 \
:katemod!katemod@katemod.tmi.twitch.tv PRIVMSG #streamer :Kappa привет\\sчат catJAM";

    #[test]
    fn parses_tags_prefix_and_trailing() {
        let line = parse(PRIVMSG).unwrap();
        assert_eq!(line.command, "PRIVMSG");
        assert_eq!(line.params, vec!["#streamer", "Kappa привет\\sчат catJAM"]);
        assert_eq!(line.tag("display-name"), "Катя");
        assert_eq!(line.tag("color"), "");
        assert_eq!(line.prefix, Some("katemod!katemod@katemod.tmi.twitch.tv"));
    }

    #[test]
    fn unescapes_tag_values() {
        assert_eq!(unescape_tag(r"a\sb\:c\\d"), r"a b;c\d");
    }

    #[test]
    fn parses_ping_and_roomstate() {
        let ping = parse("PING :tmi.twitch.tv").unwrap();
        assert_eq!((ping.command, ping.trailing()), ("PING", "tmi.twitch.tv"));
        let room = parse("@emote-only=0;room-id=12345;slow=0 :tmi.twitch.tv ROOMSTATE #streamer").unwrap();
        assert_eq!(room.tag("room-id"), "12345");
    }

    #[test]
    fn builds_chat_message() {
        let mut seventv = EmoteMap::new();
        seventv.insert("catJAM".into(), Emote { url: "https://7tv/cat".into(), zero_width: false });
        let mut badges = BadgeMap::new();
        badges.insert("moderator/1".into(), ("Moderator".into(), "https://badge/mod".into()));

        let message = to_chat_message(&parse(PRIVMSG).unwrap(), &seventv, Some(&badges));

        assert_eq!(message.id, "abc-123");
        assert_eq!(message.user_id, "2");
        assert_eq!(message.user, "Катя");
        assert_eq!(message.color, default_color("katemod"));
        assert!(!message.action);
        assert_eq!(message.badges[0].image_url.as_deref(), Some("https://badge/mod"));
        assert_eq!(message.badges[1].title, "Подписчик");
        assert!(matches!(&message.fragments[0], Fragment::Emote { name, .. } if name == "Kappa"));
        assert!(matches!(message.fragments.last(), Some(Fragment::Emote { name, .. }) if name == "catJAM"));
    }

    #[test]
    fn maps_moderation_to_deletions() {
        let clearmsg = "@login=troll;target-msg-id=abc-123 :tmi.twitch.tv CLEARMSG #s :плохое сообщение";
        assert_eq!(to_deletion(&parse(clearmsg).unwrap()), Some(ChatDeletion::Message { id: "abc-123".into() }));

        let timeout = "@ban-duration=600;room-id=1;target-user-id=42 :tmi.twitch.tv CLEARCHAT #s :troll";
        assert_eq!(to_deletion(&parse(timeout).unwrap()), Some(ChatDeletion::User { user_id: "42".into() }));

        let clear = "@room-id=1 :tmi.twitch.tv CLEARCHAT #s";
        assert_eq!(to_deletion(&parse(clear).unwrap()), Some(ChatDeletion::All));

        assert_eq!(to_deletion(&parse(PRIVMSG).unwrap()), None);
    }

    #[test]
    fn strips_me_action() {
        let raw = ":a!a@a.tmi.twitch.tv PRIVMSG #s :\u{1}ACTION танцует\u{1}";
        let message = to_chat_message(&parse(raw).unwrap(), &EmoteMap::new(), None);
        assert!(message.action);
        assert_eq!(message.text, "танцует");
    }

    /// Hits the real Twitch server: `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn joins_a_real_channel_anonymously() {
        let (mut socket, _) = connect_async(IRC_URL).await.unwrap();
        let room_id = join(&mut socket, "twitch").await.unwrap();
        assert_eq!(room_id, "12826");
    }
}
