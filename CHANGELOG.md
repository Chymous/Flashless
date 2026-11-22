# Changelog

## 1.1.2

- Viego: a spell pinged while he's possessing someone (e.g. "Morgana used Flash") now stays on his row
  when he turns back into Viego, instead of being lost. His timers and rune info follow him both ways.
- A new version now shows as an Update button at the bottom right, styled like Copy, instead of a
  pop-up at the top.

## 1.1.0

- New optional Riot API key setting (Settings > Riot API). With a key, Flashless reads each enemy's rune
  page when a game loads, so Cosmic Insight is known instead of guessed. The key is stored encrypted with
  Windows DPAPI and only sent to Riot's API.
- Get a key at developer.riotgames.com. Development keys expire after 24 hours. Riot doesn't list bot or
  practice games, so the lookup only works in real matches.
- Without a key, Cosmic Insight is still detected from early repeat pings, as before.

## 1.0.9

- The top lane quest Teleport tile now shows time left and the game time it comes back up (e.g. 4:21 and
  @22:30), like the other spells.

## 1.0.8

- Lucidity boots badge: "+10" (Ionian Boots of Lucidity) or "+20" (Crimson Lucidity) next to each enemy,
  read from the live game's item data.
- Top lane role quest Teleport: a top laner without Teleport gets a third tile for the Unleashed Teleport
  the quest grants (420 s base cooldown, summoner haste applies). Works with chat detection, the chat
  message ("tp"), compact mode and a new Top Teleport hotkey.
- Removed the level line under each name.
- Round checkboxes.

## 1.0.7

- Automatic Cosmic Insight detection: when an enemy with the Inspiration tree uses a summoner spell again
  earlier than expected, and Cosmic Insight's +18 haste explains it, Flashless marks it and corrects
  their timers.
- Boots bought while a spell is on cooldown now shorten the time left, the way the game does. Ticking
  Cosmic Insight by hand does the same.

## 1.0.6

- The opacity slider moved from the overlay footer to Settings > Appearance.

## 1.0.5

- The version number is shown in the overlay header and the tray icon tooltip.

## 1.0.4

- In-app updates: when a new version is out, the overlay shows "Update to x.y.z". One click downloads the
  setup from this repository's releases, checks its SHA-256, installs it and restarts with settings and
  shortcuts kept. Never runs during a game.
- The update check can be turned off in Settings > Appearance.
- One Flashless-Setup.exe per release that both installs and updates.

## 1.0.3

- Better chat detection: when the chat timestamp can't be read cleanly, the line is timed from when it
  appeared instead of being skipped. Chat history that pops up when chat opens is ignored.
- Flash-back alert: an enemy's Flash tile pulses when it comes back up, with an optional sound.
- Clicks pass through the overlay while League is in front, so timers can't be cleared by accident.
  Hotkeys still work. Can be turned off.
- New options in Settings > Appearance: compact mode during games, hide outside games, and Flash on the
  left, right, or in the enemy's own order.
- Call delay: a tracked spell starts 7 seconds into its cooldown to account for the time to see and call
  it, so a fresh Flash shows 4:53. Adjustable from 0 to 30 s.
- Fix: timers no longer reset mid-game when the Live Client API briefly returns an odd player list.
- Fix: the chat message keeps game-clock times if the Live Client API drops for a moment.
- Lower CPU use during games.

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
