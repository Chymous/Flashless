# Changelog

## Unreleased

- The setup downloaded by an in-app update is deleted from the temp folder on the next start.

## 1.0.6

- The opacity slider moved from the overlay footer to Settings > Appearance, so the footer has room for
  the update link.

## 1.0.5

- The version number is shown in the overlay header, the compact view's tooltip and the tray icon tooltip.

## 1.0.4

- Flashless now updates itself. When a new version is out, the overlay shows "Update to x.y.z"
  (a small icon in compact mode). Click it and Flashless downloads the new setup from this
  repository's releases, checks it against the published SHA-256, installs it and restarts with your
  settings and shortcuts kept. It never updates during a game. Right click the link to see what's new.
- "Check for a new version when Flashless starts" can be turned off in Settings > Appearance.
- Releases now carry a single Flashless-Setup.exe, which both installs and updates.

## 1.0.3

- Catches more Flashes: when League's chat time can't be read cleanly, the line is timed from the moment
  it appeared instead of being skipped. Chat history popping up when you open chat is not counted.
- Flash-back alert: an enemy's Flash tile pulses when it comes back up, with an optional Windows sound.
- Clicks pass through the overlay while League is in front, so a timer can't be cleared by accident
  mid-fight. Hotkeys still work; alt-tab to click the overlay. Can be turned off.
- New options in Settings > Appearance: compact mode during games (full view in the lobby), hide the
  overlay when no game is running, and Flash on the left / right / in the enemy's own D-F order.
- A tracked spell starts 7 seconds into its cooldown (time to see it and call it), so a fresh Flash
  shows 4:53. Adjustable from 0 to 30 s; applies to clicks, hotkeys and chat detection.
- Chat timestamps read reliably with a small chat box (`1 5:44`, `15.'44`, `I I .'56` and similar).
- Fix: timers no longer reset mid-game when the Live Client API briefly lists every player or a
  placeholder player (bot games).
- Fix: the chat message keeps game-clock times if the Live Client API drops for a moment.
- Lighter while you play: Windows hands over at most one captured frame a second, and the Live Client
  API is polled every 5 s instead of every second.
- `chat.log` keeps only the last three games.

## 1.0.2

- Real summoner spell icons, with a League-style cooldown sweep over them.
- Compact mode: a small HUD with just lanes and spell icons (overlay button, tray menu or hotkey).
- Chat reading crops the chat area on the GPU and only runs OCR when new text appears.
- Ultimate tracking removed (against Riot's third-party rules).

## 1.0.1

- Hotkeys can be mouse buttons 3, 4 and 5.
- Updater: `Flashless-Update-x.y.z.exe` updates an existing install in place and shows what's new.

## 1.0.0

First release of Flashless, a rewrite of an earlier C# prototype in Rust.

- Overlay with each enemy's summoner spells, in lane order (Top, Jg, Mid, ADC, Sup).
- Roster, levels, items and spells from League's Live Client API.
- Cooldowns with summoner spell haste (Ionian Boots, Crimson Lucidity, Cosmic Insight, ARAM) and
  Unleashed Teleport.
- Chat reading with Windows OCR: "Enemy used Flash" lines start the timer. Draw or auto-detect the
  chat area in Settings.
- One-line chat message, Flash first, with lane names. Copy, auto-copy, or type it into chat without
  sending (then select and copy the chat line).
- Keyboard hotkeys, light and dark themes, tray icon.
- Per-user installer: Start menu (Windows search) and desktop shortcuts, Installed apps entry, Win+R.
