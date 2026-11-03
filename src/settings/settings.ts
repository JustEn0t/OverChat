import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { randomMessage } from "../shared/testMessages";
import { EVENTS, type AccountStatus, type ChatStatus, type Settings } from "../shared/types";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const visible = $<HTMLInputElement>("visible");
const clickThrough = $<HTMLInputElement>("click-through");
const clickError = $<HTMLParagraphElement>("click-error");
const gameMode = $<HTMLInputElement>("game-mode");
const opacity = $<HTMLInputElement>("opacity");
const opacityValue = $("opacity-value");
const chatOpacity = $<HTMLInputElement>("chat-opacity");
const chatOpacityValue = $("chat-opacity-value");
const fontFamily = $<HTMLSelectElement>("font-family");
const fontSize = $<HTMLInputElement>("font-size");
const fontSizeValue = $("font-size-value");
const compact = $<HTMLInputElement>("compact");
const animatedEmotes = $<HTMLInputElement>("animated-emotes");
const maxMessages = $<HTMLInputElement>("max-messages");
const maxMessagesValue = $("max-messages-value");
const fadeAfter = $<HTMLSelectElement>("fade-after");
const hideCommands = $<HTMLInputElement>("hide-commands");
const hotkeyError = $<HTMLParagraphElement>("hotkey-error");
const demo = $<HTMLInputElement>("demo");
const testButton = $<HTMLButtonElement>("test-message");
const autostart = $<HTMLInputElement>("autostart");
const autostartError = $<HTMLParagraphElement>("autostart-error");
const closeToTray = $<HTMLInputElement>("close-to-tray");
const fullscreenWarning = $("fullscreen-warning");

const chatDot = $("chat-dot");
const chatText = $("chat-text");
const chatError = $<HTMLParagraphElement>("chat-error");
const channelForm = $<HTMLFormElement>("channel-form");
const channelInput = $<HTMLInputElement>("channel");
const channelButton = $<HTMLButtonElement>("channel-button");

const accountBox = $<HTMLDetailsElement>("account");
const twitchDot = $("twitch-dot");
const twitchText = $("twitch-text");
const twitchError = $<HTMLParagraphElement>("twitch-error");
const signedOutBox = $("twitch-signed-out");
const awaitingBox = $("twitch-awaiting");
const signedInBox = $("twitch-signed-in");
const clientIdField = $("client-id-field");
const clientIdInput = $<HTMLInputElement>("client-id");
const loginButton = $<HTMLButtonElement>("login");
const userCode = $("user-code");

type HotkeyField = "hotkeyClickThrough" | "hotkeyToggleOverlay";

let settings: Settings;
let builtinClientId: string | null = null;
let wasConnected = false;
let chatActive = false;
let saveTimer: number | undefined;

function showError(el: HTMLElement, message: string | null): void {
  el.hidden = !message;
  el.textContent = message ?? "";
}

function pluralEmotes(n: number): string {
  const mod10 = n % 10;
  const mod100 = n % 100;
  if (mod10 === 1 && mod100 !== 11) return `${n} смайлик`;
  if (mod10 >= 2 && mod10 <= 4 && (mod100 < 12 || mod100 > 14)) return `${n} смайлика`;
  return `${n} смайликов`;
}

// ── Settings (stored by Rust in settings.json) ───────────────────────────

/** Sends settings to Rust. Sliders pass `debounce` so dragging doesn't write the file 60 times a second. */
function save(patch: Partial<Settings>, debounce = false): Promise<void> {
  settings = { ...settings, ...patch };
  window.clearTimeout(saveTimer);
  const send = async () => {
    try {
      settings = await invoke<Settings>("update_settings", { settings });
      showError(hotkeyError, null);
    } catch (err) {
      // Only hotkeys can be rejected (busy or invalid combination); restore what is really saved.
      showError(hotkeyError, String(err));
      settings = await invoke<Settings>("get_settings");
    }
    renderSettings();
  };
  if (!debounce) return send();
  return new Promise((resolve) => {
    saveTimer = window.setTimeout(() => void send().then(resolve), 120);
  });
}

function setValue(el: HTMLInputElement | HTMLSelectElement, value: string): void {
  // Don't fight the user over the control they're dragging right now.
  if (document.activeElement !== el) el.value = value;
}

function renderSettings(): void {
  clickThrough.checked = settings.clickThrough;
  gameMode.checked = settings.gameMode;
  compact.checked = settings.compact;
  animatedEmotes.checked = settings.animatedEmotes;
  hideCommands.checked = settings.hideCommands;
  demo.checked = settings.demoStream;
  closeToTray.checked = settings.closeToTray;

  setValue(opacity, String(settings.bgOpacity));
  opacityValue.textContent = `${settings.bgOpacity}%`;
  setValue(chatOpacity, String(settings.chatOpacity));
  chatOpacityValue.textContent = `${settings.chatOpacity}%`;
  setValue(fontFamily, settings.fontFamily);
  setValue(fontSize, String(settings.fontSize));
  fontSizeValue.textContent = `${settings.fontSize} px`;
  setValue(maxMessages, String(settings.maxMessages));
  maxMessagesValue.textContent = String(settings.maxMessages);
  setValue(fadeAfter, String(settings.fadeAfterSecs));
  setValue(clientIdInput, settings.clientId);

  renderHotkeys();
}

const range = (
  el: HTMLInputElement,
  key: "bgOpacity" | "chatOpacity" | "fontSize" | "maxMessages",
  label: HTMLElement,
  unit = "",
) =>
  el.addEventListener("input", () => {
    label.textContent = `${el.value}${unit}`;
    void save({ [key]: Number(el.value) }, true);
  });

range(opacity, "bgOpacity", opacityValue, "%");
range(chatOpacity, "chatOpacity", chatOpacityValue, "%");
range(fontSize, "fontSize", fontSizeValue, " px");
range(maxMessages, "maxMessages", maxMessagesValue);

fontFamily.addEventListener("change", () => void save({ fontFamily: fontFamily.value }));
fadeAfter.addEventListener("change", () => void save({ fadeAfterSecs: Number(fadeAfter.value) }));
gameMode.addEventListener("change", () => void save({ gameMode: gameMode.checked }));
compact.addEventListener("change", () => void save({ compact: compact.checked }));
animatedEmotes.addEventListener("change", () => void save({ animatedEmotes: animatedEmotes.checked }));
hideCommands.addEventListener("change", () => void save({ hideCommands: hideCommands.checked }));
demo.addEventListener("change", () => void save({ demoStream: demo.checked }));

testButton.addEventListener("click", () => void emit(EVENTS.chatMessage, randomMessage()));

// ── App: autostart, tray, quit ───────────────────────────────────────────

closeToTray.addEventListener("change", () => void save({ closeToTray: closeToTray.checked }));

autostart.addEventListener("change", async () => {
  try {
    await invoke("set_autostart", { enabled: autostart.checked });
    showError(autostartError, null);
  } catch (err) {
    autostart.checked = !autostart.checked;
    showError(autostartError, String(err));
  }
});

$("quit").addEventListener("click", () => void invoke("quit_app"));

void listen<boolean>(EVENTS.exclusiveFullscreen, (e) => (fullscreenWarning.hidden = !e.payload));

// ── Overlay window ───────────────────────────────────────────────────────

clickThrough.addEventListener("change", async () => {
  try {
    await invoke("set_click_through", { enabled: clickThrough.checked });
    showError(clickError, null);
  } catch (err) {
    clickThrough.checked = !clickThrough.checked;
    showError(clickError, `Не удалось переключить режим: ${String(err)}`);
  }
});

visible.addEventListener("change", () => {
  invoke("set_overlay_visible", { visible: visible.checked }).catch((err) => {
    visible.checked = !visible.checked;
    showError(clickError, `Не удалось ${visible.checked ? "скрыть" : "показать"} чат: ${String(err)}`);
  });
});

void listen<boolean>(EVENTS.clickThrough, (e) => (clickThrough.checked = e.payload));
void listen<boolean>(EVENTS.overlayVisibility, (e) => (visible.checked = e.payload));

// ── Hotkeys ──────────────────────────────────────────────────────────────

const hotkeyButtons = document.querySelectorAll<HTMLButtonElement>("button.hotkey");
let recording: { button: HTMLButtonElement; field: HotkeyField } | null = null;

function renderHotkeys(): void {
  for (const button of hotkeyButtons) {
    if (recording?.button === button) continue;
    button.textContent = settings[button.dataset.hotkey as HotkeyField] || "не задано";
  }
  for (const hint of document.querySelectorAll<HTMLElement>("kbd.hotkey-hint")) {
    hint.textContent = settings[hint.dataset.hotkey as HotkeyField];
  }
}

/** KeyboardEvent.code → the key name Rust's shortcut parser understands. */
function keyName(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit\d$/.test(code)) return code.slice(5);
  if (/^(F\d{1,2}|Numpad\d|Space|Insert|Home|End|PageUp|PageDown|Backquote|Minus|Equal|BracketLeft|BracketRight|Semicolon|Quote|Comma|Period|Slash|Backslash|ArrowUp|ArrowDown|ArrowLeft|ArrowRight)$/.test(code)) {
    return code;
  }
  return null;
}

function startRecording(button: HTMLButtonElement): void {
  if (recording) void finishRecording(null);
  recording = { button, field: button.dataset.hotkey as HotkeyField };
  button.classList.add("recording");
  button.textContent = "Нажмите сочетание…";
  showError(hotkeyError, null);
  // Otherwise our own hotkeys would swallow the key press we want to record.
  void invoke("pause_hotkeys");
}

async function finishRecording(value: string | null): Promise<void> {
  const current = recording;
  if (!current) return;
  recording = null;
  current.button.classList.remove("recording");
  if (value !== null && value !== settings[current.field]) {
    await save({ [current.field]: value });
  }
  await invoke("resume_hotkeys");
  renderHotkeys();
}

for (const button of hotkeyButtons) {
  button.addEventListener("click", () => startRecording(button));
}

window.addEventListener(
  "keydown",
  (event) => {
    if (!recording) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.code === "Escape") return void finishRecording(null);
    if (event.code === "Backspace" || event.code === "Delete") return void finishRecording("");

    const key = keyName(event.code);
    if (!key) return; // A modifier on its own, or a key we can't bind: keep listening.

    const modifiers = [
      event.ctrlKey && "Ctrl",
      event.altKey && "Alt",
      event.shiftKey && "Shift",
      event.metaKey && "Super",
    ].filter(Boolean) as string[];
    if (modifiers.length === 0 && !/^F\d{1,2}$/.test(key)) {
      showError(hotkeyError, "Добавьте Ctrl, Alt или Shift — одиночная клавиша будет мешать в игре.");
      return;
    }
    void finishRecording([...modifiers, key].join("+"));
  },
  true,
);

window.addEventListener("blur", () => void finishRecording(null));

// ── Chat ─────────────────────────────────────────────────────────────────

function renderChat(status: ChatStatus): void {
  chatActive = status.state !== "disconnected" && status.state !== "error";
  channelButton.textContent = chatActive ? "Отключить" : "Подключить";
  channelButton.className = chatActive ? "secondary" : "";

  let dot = "";
  let text = "";
  let error: string | null = null;
  switch (status.state) {
    case "disconnected":
      text = "Чат не подключён";
      break;
    case "connecting":
      dot = "busy";
      text = `Подключаемся к чату ${status.channel}…`;
      break;
    case "connected": {
      dot = "ok";
      const seventv = status.seventvEmotes > 0 ? `7TV: ${pluralEmotes(status.seventvEmotes)}` : "7TV: нет смайликов";
      text = `Чат ${status.channel} подключён · ${seventv}`;
      break;
    }
    case "reconnecting":
      dot = "busy";
      text = `Переподключение к ${status.channel}…`;
      error = status.message;
      break;
    case "error":
      dot = "bad";
      text = "Чат не подключён";
      error = status.message;
      break;
  }
  chatDot.className = `dot ${dot}`;
  chatText.textContent = text;
  showError(chatError, error);

  // Real chat replaces the demo stream; the user can switch it back on.
  // (A status event can arrive before settings have loaded.)
  if (status.state === "connected" && !wasConnected && settings?.demoStream) {
    void save({ demoStream: false });
  }
  wasConnected = status.state === "connected";
}

channelForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  if (chatActive) {
    void invoke("chat_disconnect");
    return;
  }
  try {
    channelInput.value = await invoke<string>("chat_connect", { channel: channelInput.value });
  } catch (err) {
    renderChat({ state: "error", message: String(err) });
  }
});

void listen<ChatStatus>(EVENTS.chatStatus, (e) => renderChat(e.payload));

// ── Optional sign-in (badge images) ──────────────────────────────────────

function renderAccount(status: AccountStatus): void {
  signedOutBox.hidden = !(status.state === "signedOut" || status.state === "error");
  awaitingBox.hidden = status.state !== "awaitingUser";
  signedInBox.hidden = status.state !== "signedIn";
  clientIdField.hidden = builtinClientId !== null;
  loginButton.disabled = false;

  let dot = "";
  let text = "";
  let error: string | null = null;
  switch (status.state) {
    case "signedOut":
      text = "Вход не выполнен — значки буквами";
      break;
    case "checking":
      dot = "busy";
      text = "Проверяем вход…";
      break;
    case "awaitingUser":
      dot = "busy";
      text = "Ожидаем подтверждения в браузере…";
      userCode.textContent = status.userCode;
      accountBox.open = true;
      break;
    case "signedIn":
      dot = "ok";
      text = `Вход: ${status.login} — значки картинками`;
      break;
    case "error":
      dot = "bad";
      text = "Вход не выполнен — значки буквами";
      error = status.message;
      accountBox.open = true;
      break;
  }
  twitchDot.className = `dot ${dot}`;
  twitchText.textContent = text;
  showError(twitchError, error);
}

loginButton.addEventListener("click", async () => {
  loginButton.disabled = true;
  try {
    await invoke("account_login", { clientId: builtinClientId ?? clientIdInput.value });
  } catch (err) {
    renderAccount({ state: "error", message: String(err) });
  }
});

$("reopen").addEventListener("click", () => {
  invoke("account_open_verification").catch((err) => showError(twitchError, String(err)));
});
$("cancel").addEventListener("click", () => void invoke("account_cancel"));
$("logout").addEventListener("click", () => void invoke("account_logout"));

void listen<AccountStatus>(EVENTS.accountStatus, (e) => renderAccount(e.payload));

// ── Startup ──────────────────────────────────────────────────────────────

/** Versions up to 0.3 kept settings in localStorage; move them into settings.json once. */
async function migrateLocalStorage(): Promise<void> {
  const keys = ["overchat.settings", "overchat.channel", "overchat.twitchClientId"];
  try {
    const [old, channel, clientId] = keys.map((k) => localStorage.getItem(k));
    if (!old && !channel && !clientId) return;
    const patch: Partial<Settings> = {};
    if (old) {
      const parsed = JSON.parse(old);
      if (typeof parsed.bgOpacity === "number") patch.bgOpacity = parsed.bgOpacity;
      if (typeof parsed.demoStream === "boolean") patch.demoStream = parsed.demoStream;
    }
    if (clientId && !settings.clientId) patch.clientId = clientId;
    if (Object.keys(patch).length > 0) await save(patch);
    if (channel && !settings.channel) {
      channelInput.value = await invoke<string>("chat_connect", { channel });
    }
    keys.forEach((k) => localStorage.removeItem(k));
  } catch {
    // Old data is optional; defaults are fine.
  }
}

void (async () => {
  settings = await invoke<Settings>("get_settings");
  renderSettings();
  channelInput.value = settings.channel;
  void listen<Settings>(EVENTS.settingsChanged, (e) => {
    settings = e.payload;
    renderSettings();
  });

  visible.checked = await invoke<boolean>("get_overlay_visible");
  autostart.checked = await invoke<boolean>("get_autostart");
  fullscreenWarning.hidden = !(await invoke<boolean>("get_exclusive_fullscreen"));
  showError(hotkeyError, await invoke<string | null>("hotkey_status"));

  builtinClientId = await invoke<string | null>("account_builtin_client_id");
  renderAccount(await invoke<AccountStatus>("account_status"));
  renderChat(await invoke<ChatStatus>("chat_status"));

  await migrateLocalStorage();
})();
