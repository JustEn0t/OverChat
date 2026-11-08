import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { randomMessage } from "../shared/testMessages";
import { EVENTS, type Badge, type ChatDeletion, type ChatMessage, type Fragment, type Settings } from "../shared/types";

const DEMO_INTERVAL_MS = 2500;
const FADE_CHECK_MS = 1000;
/** Must match the transition in overlay.css. */
const FADE_OUT_MS = 600;
/** How close to the bottom (px) still counts as "following the chat". */
const STICK_THRESHOLD = 40;

/** Letter fallback when there is no badge image (test messages, badges failed to load). */
const BADGE_LETTERS: Record<string, string> = {
  broadcaster: "B",
  moderator: "M",
  lead_moderator: "M",
  vip: "V",
  subscriber: "S",
  founder: "S",
};

const list = document.getElementById("messages") as HTMLUListElement;
const handleText = document.getElementById("handle-text") as HTMLElement;
let settings: Settings | null = null;
let demoTimer: number | undefined;
/** False while the user has scrolled up to read older messages (only possible without click-through). */
let followChat = true;

function formatTime(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString("ru-RU", { hour: "2-digit", minute: "2-digit" });
}

/** Twitch lets users pick very dark name colors; lighten them so they stay readable over a game. */
function readableColor(hex: string): string {
  const match = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!match) return "#ffffff";
  const n = parseInt(match[1], 16);
  let [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  const luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  if (luminance < 0.45) {
    const mix = 0.45 - luminance + 0.15;
    [r, g, b] = [r, g, b].map((c) => Math.round(c + (255 - c) * mix));
  }
  return `rgb(${r}, ${g}, ${b})`;
}

function renderBadge(badge: Badge): HTMLElement | null {
  if (badge.imageUrl) {
    const img = document.createElement("img");
    img.className = "badge-img";
    img.src = badge.imageUrl;
    img.alt = badge.title;
    img.title = badge.title;
    return img;
  }
  const letter = BADGE_LETTERS[badge.kind];
  if (!letter) return null;
  const el = document.createElement("span");
  el.className = `badge ${badge.kind}`;
  el.textContent = letter;
  el.title = badge.title;
  return el;
}

// Chat text is untrusted, so the DOM is built with textContent only — never innerHTML.
function renderMessage(msg: ChatMessage): void {
  if (settings?.hideCommands && msg.text.trimStart().startsWith("!")) return;

  const item = document.createElement("li");
  item.className = "msg";
  item.dataset.id = msg.id;
  item.dataset.userId = msg.userId;
  item.dataset.arrived = String(Date.now());

  const time = document.createElement("span");
  time.className = "time";
  time.textContent = formatTime(msg.timestamp);
  item.append(time);

  for (const badge of msg.badges) {
    const el = renderBadge(badge);
    if (el) item.append(el);
  }

  const color = readableColor(msg.color);
  const user = document.createElement("span");
  user.className = "user";
  user.style.color = color;
  user.textContent = msg.user;

  const body = document.createElement("span");
  if (msg.action) {
    body.className = "action";
    body.style.color = color;
  }
  renderFragments(body, msg.fragments);

  item.append(user, document.createTextNode(msg.action ? " " : ": "), body);

  // Fast chats bring dozens of messages a second: add them to the page once per frame,
  // not one by one. While the overlay is hidden frames pause, so keep only what would fit.
  pending.push(item);
  if (pending.length > maxMessages()) pending.splice(0, pending.length - maxMessages());
  scheduleFlush();
}

function maxMessages(): number {
  return settings?.maxMessages ?? 50;
}

let pending: HTMLElement[] = [];
let flushScheduled = false;

function scheduleFlush(): void {
  if (flushScheduled) return;
  flushScheduled = true;
  requestAnimationFrame(() => {
    flushScheduled = false;
    if (pending.length > 0) {
      list.append(...pending);
      pending = [];
      trimMessages();
    }
    if (followChat) list.scrollTop = list.scrollHeight;
  });
}

function trimMessages(): void {
  while (list.children.length > maxMessages()) list.firstElementChild?.remove();
  updateEmpty();
}

/** Lets a fully faded-out chat leave no background box over the game. */
function updateEmpty(): void {
  document.body.classList.toggle("empty", list.querySelector(".msg:not(.fading)") === null);
}

/** Coalesced into the next frame: many emotes finishing loading at once cost one layout. */
function scrollToBottom(): void {
  if (followChat) scheduleFlush();
}

list.addEventListener("scroll", () => {
  followChat = list.scrollHeight - list.scrollTop - list.clientHeight < STICK_THRESHOLD;
});

function removeMessages(deletion: ChatDeletion): void {
  const matches = (item: HTMLElement) =>
    deletion.kind === "all" ||
    (deletion.kind === "message" && item.dataset.id === deletion.id) ||
    (deletion.kind === "user" && item.dataset.userId === deletion.userId);
  for (const item of list.querySelectorAll<HTMLElement>(".msg")) {
    if (matches(item)) item.remove();
  }
  pending = pending.filter((item) => !matches(item));
  updateEmpty();
}

function fadeOldMessages(): void {
  const fadeAfter = settings?.fadeAfterSecs ?? 0;
  if (fadeAfter <= 0) return;
  const cutoff = Date.now() - fadeAfter * 1000;
  for (const item of list.querySelectorAll<HTMLElement>(".msg:not(.fading)")) {
    if (Number(item.dataset.arrived) > cutoff) break; // Newer messages come later in the list.
    item.classList.add("fading");
    window.setTimeout(() => item.remove(), FADE_OUT_MS);
  }
  updateEmpty();
}

/** Both CDNs serve a still first frame of every emote; used when animation is switched off. */
function emoteUrl(url: string): string {
  if (settings?.animatedEmotes !== false) return url;
  return url
    .replace(/(cdn\.7tv\.app\/emote\/[^/]+\/\dx)\.webp$/, "$1_static.webp")
    .replace(/(\/emoticons\/v2\/[^/]+)\/default\//, "$1/static/");
}

function updateEmoteAnimation(): void {
  for (const img of list.querySelectorAll<HTMLImageElement>("img.emote")) {
    const src = emoteUrl(img.dataset.url ?? img.src);
    if (img.src !== src) img.src = src;
  }
}

function renderFragments(target: HTMLElement, fragments: Fragment[]): void {
  let lastEmote: HTMLElement | null = null;
  for (const fragment of fragments) {
    if (fragment.type === "text") {
      target.append(document.createTextNode(fragment.text));
      lastEmote = null;
      continue;
    }

    const img = document.createElement("img");
    img.className = "emote";
    img.dataset.url = fragment.url;
    img.src = emoteUrl(fragment.url);
    img.alt = fragment.name;
    img.title = fragment.name;
    // Emotes change the message height once loaded; keep the newest message in view.
    img.addEventListener("load", scrollToBottom, { once: true });
    // A broken emote URL falls back to its name, like in Twitch chat.
    img.addEventListener("error", () => img.replaceWith(document.createTextNode(fragment.name)), { once: true });

    // Zero-width 7TV emotes (PETPET, RainTime…) are drawn on top of the previous emote.
    if (fragment.zeroWidth && lastEmote) {
      img.classList.add("zero-width");
      if (!lastEmote.classList.contains("emote-stack")) {
        const stack = document.createElement("span");
        stack.className = "emote-stack";
        lastEmote.replaceWith(stack);
        stack.append(lastEmote);
        lastEmote = stack;
      }
      lastEmote.append(img);
      continue;
    }

    target.append(img);
    lastEmote = img;
  }
}

function applySettings(next: Settings): void {
  const animationChanged = settings !== null && settings.animatedEmotes !== next.animatedEmotes;
  settings = next;
  if (animationChanged) updateEmoteAnimation();
  const root = document.documentElement.style;
  root.setProperty("--bg-alpha", String(next.bgOpacity / 100));
  root.setProperty("--chat-alpha", String(next.chatOpacity / 100));
  root.setProperty("--font-size", `${next.fontSize}px`);
  root.setProperty("--font-family", `"${next.fontFamily.replace(/"/g, "")}", "Segoe UI", sans-serif`);
  document.body.classList.toggle("compact", next.compact);
  handleText.textContent = next.hotkeyClickThrough
    ? `Перетащите окно · ${next.hotkeyClickThrough} — сквозные клики`
    : "Перетащите окно";
  setClickThrough(next.clickThrough);
  trimMessages();

  if (next.demoStream && demoTimer === undefined) {
    demoTimer = window.setInterval(() => renderMessage(randomMessage()), DEMO_INTERVAL_MS);
  } else if (!next.demoStream && demoTimer !== undefined) {
    window.clearInterval(demoTimer);
    demoTimer = undefined;
  }
}

function setClickThrough(enabled: boolean): void {
  document.body.classList.toggle("click-through", enabled);
  // With click-through on the user can't scroll, so always follow the newest messages.
  if (enabled) {
    followChat = true;
    scrollToBottom();
  }
}

document.getElementById("resize-grip")?.addEventListener("mousedown", (event) => {
  event.preventDefault();
  void getCurrentWindow().startResizeDragging("SouthEast");
});

void (async () => {
  await Promise.all([
    listen<Settings>(EVENTS.settingsChanged, (e) => applySettings(e.payload)),
    listen<ChatMessage>(EVENTS.chatMessage, (e) => renderMessage(e.payload)),
    listen<ChatDeletion>(EVENTS.chatDelete, (e) => removeMessages(e.payload)),
    listen<boolean>(EVENTS.clickThrough, (e) => setClickThrough(e.payload)),
    listen<boolean>(EVENTS.overlayVisibility, (e) => {
      document.body.classList.toggle("overlay-hidden", !e.payload);
      if (e.payload) scheduleFlush();
    }),
  ]);
  const initial = await invoke<Settings>("get_settings");
  applySettings(initial);
  if (initial.demoStream) for (let i = 0; i < 6; i++) renderMessage(randomMessage());
  updateEmpty();
  document.body.classList.toggle("overlay-hidden", !(await invoke<boolean>("get_overlay_visible")));
})();

window.setInterval(fadeOldMessages, FADE_CHECK_MS);
