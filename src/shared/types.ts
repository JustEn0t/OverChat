export interface Badge {
  /** Twitch badge set id: "broadcaster", "moderator", "vip", "subscriber", … */
  kind: string;
  title: string;
  /** Badge image; only when signed in to Twitch, otherwise the overlay draws a letter. */
  imageUrl?: string | null;
}

export type Fragment =
  | { type: "text"; text: string }
  /** Twitch or 7TV emote. Zero-width emotes are drawn on top of the previous emote. */
  | { type: "emote"; name: string; url: string; zeroWidth: boolean };

/** Same shape as `ChatMessage` in src-tauri/src/twitch/message.rs. */
export interface ChatMessage {
  id: string;
  userId: string;
  user: string;
  color: string;
  text: string;
  /** `/me` message, shown in the user's color. */
  action: boolean;
  fragments: Fragment[];
  badges: Badge[];
  timestamp: number;
}

/** Moderation: mirrors `ChatDeletion` in src-tauri/src/twitch/message.rs. */
export type ChatDeletion =
  | { kind: "message"; id: string }
  | { kind: "user"; userId: string }
  | { kind: "all" };

/** Stored by Rust in settings.json; mirrors `Settings` in src-tauri/src/settings.rs. */
export interface Settings {
  /** Chat background opacity, 0–100 %. Text stays fully opaque for readability. */
  bgOpacity: number;
  /** Opacity of the messages themselves (text, badges, emotes), 10–100 %. */
  chatOpacity: number;
  fontFamily: string;
  fontSize: number;
  /** No timestamps, tighter lines, smaller emotes. */
  compact: boolean;
  maxMessages: number;
  /** Hide messages this many seconds after they arrive; 0 = never. */
  fadeAfterSecs: number;
  /** Hide chat commands like `!discord`. */
  hideCommands: boolean;
  /** Off: emotes show their first frame, which saves GPU work on fast chats. */
  animatedEmotes: boolean;
  /** Overlay generates fake messages so it can be checked over a game. */
  demoStream: boolean;
  clickThrough: boolean;
  /** Periodically puts the chat back on top of borderless games. */
  gameMode: boolean;
  hotkeyClickThrough: string;
  hotkeyToggleOverlay: string;
  /** Closing the settings window hides it to the tray instead of quitting. */
  closeToTray: boolean;
  channel: string;
  clientId: string;
}

/** Mirrors `ChatStatus` in src-tauri/src/twitch/mod.rs. */
export type ChatStatus =
  | { state: "disconnected" }
  | { state: "connecting"; channel: string }
  | { state: "connected"; channel: string; seventvEmotes: number; badgeImages: boolean }
  | { state: "reconnecting"; channel: string; message: string }
  | { state: "error"; message: string };

/** Mirrors `AccountStatus` in src-tauri/src/twitch/mod.rs. */
export type AccountStatus =
  | { state: "signedOut" }
  | { state: "checking" }
  | { state: "awaitingUser"; userCode: string; verificationUri: string }
  | { state: "signedIn"; login: string }
  | { state: "error"; message: string };

/** Event names shared by both windows and the Rust side (see src-tauri/src). */
export const EVENTS = {
  settingsChanged: "settings-changed",
  chatMessage: "chat-message",
  chatDelete: "chat-delete",
  clickThrough: "click-through-changed",
  overlayVisibility: "overlay-visibility",
  exclusiveFullscreen: "exclusive-fullscreen",
  chatStatus: "chat-status",
  accountStatus: "account-status",
} as const;
