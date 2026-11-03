import type { Badge, ChatMessage, Fragment } from "./types";

const BROADCASTER: Badge = { kind: "broadcaster", title: "Стример" };
const MODERATOR: Badge = { kind: "moderator", title: "Модератор" };
const VIP: Badge = { kind: "vip", title: "VIP" };
const SUBSCRIBER: Badge = { kind: "subscriber", title: "Подписчик" };

const USERS: { name: string; color: string; badges: Badge[] }[] = [
  { name: "StreamerName", color: "#FF4F4F", badges: [BROADCASTER] },
  { name: "ModeratorKate", color: "#00C853", badges: [MODERATOR, SUBSCRIBER] },
  { name: "vip_artem", color: "#E05BFF", badges: [VIP] },
  { name: "NightOwl_92", color: "#1E90FF", badges: [SUBSCRIBER] },
  { name: "pixelgamer", color: "#FFB300", badges: [] },
  { name: "SilentViewer", color: "#00BFA5", badges: [] },
  { name: "DimaPro", color: "#FF7043", badges: [SUBSCRIBER] },
  { name: "lurker_404", color: "#0000FF", badges: [] },
];

const TEXTS = [
  "Привет, чат! 👋",
  "Какой это уровень сложности?",
  "GG, отличный раунд",
  "Слева кто-то был, аккуратнее",
  "!discord",
  "Ахаха, вот это поворот",
  "Сколько часов уже в игре?",
  "Подписался, давно смотрю стримы 💜",
  "Можно проверить, как читается длинное сообщение поверх яркой сцены игры — переносится ли текст и не теряется ли он на светлом фоне?",
  "lol",
  "Какие настройки графики стоят?",
  "Спасибо за стрим!",
];

const text = (t: string): Fragment => ({ type: "text", text: t });
const emote = (name: string, url: string, zeroWidth = false): Fragment => ({ type: "emote", name, url, zeroWidth });

// Real Twitch and 7TV global emotes, so the test shows how emotes look in the overlay.
const KAPPA = emote("Kappa", "https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/2.0");
const RECKH = emote("reckH", "https://cdn.7tv.app/emote/01FGH8NE3800064MEQW00DNBNG/2x.webp");
const PETPET = emote("PETPET", "https://cdn.7tv.app/emote/01FSNK0GP000038GDBZ91HT6SQ/2x.webp", true);

const EMOTE_MESSAGES: Fragment[][] = [
  [text("Хорош "), KAPPA],
  [RECKH, text(" вот это поворот")],
  [text("гладим стримера "), KAPPA, PETPET],
];

let counter = 0;

function pick<T>(items: T[]): T {
  return items[Math.floor(Math.random() * items.length)];
}

export function randomMessage(): ChatMessage {
  const user = pick(USERS);
  const fragments = Math.random() < 0.35 ? pick(EMOTE_MESSAGES) : [text(pick(TEXTS))];
  counter += 1;
  return {
    id: `test-${Date.now()}-${counter}`,
    userId: `test-${user.name}`,
    user: user.name,
    color: user.color,
    badges: user.badges,
    text: fragments.map((f) => (f.type === "text" ? f.text : f.name)).join(""),
    action: false,
    fragments,
    timestamp: Date.now(),
  };
}
