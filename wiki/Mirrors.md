# Mirrors

Downloads rotate across every enabled mirror.

## Built-in mirrors

| Mirror | Host | Default | Notes |
|---|---|---|---|
| osu!direct | `osu.direct` | on | |
| Nerinyan | `api.nerinyan.moe` | on | |
| Sayobot | `dl.sayobot.cn` | on | |
| Nekoha | `mirror.nekoha.moe` | on | Also serves the size estimates for find results |
| Beatconnect | `beatconnect.io` | on | |
| osu!dl | `osudl.org` | on | Covers ranked / approved / loved sets only; rotation backfills the rest |
| catboy.best | `catboy.best` | on | |
| Hinamizawa | `mirror.hinamizawa.ai` | off | Races the other mirrors server-side, enable if you're hitting rate limits |
| nzbasic CDN | `direct.nzbasic.com` | off | A solo developer's free instance; coverage is what its backend has cached |
| osu! official | `osu.ppy.sh` | off | Needs osu! login (below) |

## Managing mirrors

The config tab is the mirror editor: toggle each built-in on or off, reorder the try-order with <kbd>⇧</kbd>+<kbd>↑</kbd>/<kbd>↓</kbd>, or press <kbd>r</kbd> to re-probe latency (shown per mirror next to each row). The get maps tab shows the enabled count with the latency range.

## Rate-limit handling

- Requests to each mirror are spaced out; the spacing widens when a mirror pushes back, then decays once it calms down.
- A throttled mirror sits out its cooldown while the rest keep downloading. Per-map countdowns show in the open run.
- A map that hits a limit goes back in the queue rather than getting dropped. One that stays throttled past the auto-defer delay (60s of actual waiting by default, `download.rate_limit_skip_secs`) re-queues on its own.
- In an open run, <kbd>s</kbd> defers the currently stuck maps yourself; <kbd>S</kbd> drops them for the rest of the run.

## Custom mirrors

Add your own on the config tab: each URL template must contain the `{id}` placeholder (for example `https://example.com/d/{id}`). A new empty row appears as you type; clearing a row removes it. Custom mirrors are tried after the built-ins, in list order. The matching config key is `mirror.urls` ([Configuration](Configuration#mirror)).

## Logging in with your osu! account

Logging in is optional; its only purpose is enabling the **osu! official** mirror. The account row on the config tab opens a login panel docked on the right, where you enter your osu! username and password (masked). It closes on <kbd>esc</kbd> or a tab switch.

The login goes through osu!lazer's first-party client, the only client allowed to request beatmap-download privilege. If osu! needs to verify a new device, the panel prompts for the emailed code. Your password is sent only to `osu.ppy.sh` and never stored; only the resulting token lives in a local `auth.json`.

> [!WARNING]
> This uses osu!lazer's first-party login, an unofficial grey area. The official mirror is rate-limited (roughly 10 to 20 downloads per hour) and stays off by default. Keep it as a last-resort source. Requests to osu! are throttled automatically (about one per second, shared across all download threads) to stay within its general API rate; the hourly download cap still applies and shows up as a temporary rate-limit when reached.
