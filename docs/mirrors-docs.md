# Complete osu! Mirror APIs Documentation (November 2025)

## Mirror Status Overview

| Mirror | Status | Former Names | API Access | Notes |
|--------|--------|--------------|------------|-------|
| **osu.direct** | ✅ Active | Kitsu | Free, v1 & v2 API | Rebranded from Kitsu in 2024 |
| **Nerinyan** | ✅ Active | - | Free, Full API | Fast sync with official osu! |
| **Sayobot** | ✅ Active | - | Free | Chinese mirror |
| **Mino (catboy.best)** | ✅ Active | - | Free, Full API | Multi-region, 100% osu!direct compatible |

---

## 1. Nerinyan (api.nerinyan.moe)

**Website:** https://nerinyan.moe  
**API Base:** https://api.nerinyan.moe  
**Status:** Active, Free  

### Download Endpoints

#### Download Beatmapset (Full Package)
```
GET https://api.nerinyan.moe/d/{beatmapset_id}
```

**Parameters:**
- `nv=1` - Download without video (optional)

**Examples:**
```bash
# With video
curl "https://api.nerinyan.moe/d/1" -o beatmapset.osz

# Without video  
curl "https://api.nerinyan.moe/d/1?nv=1" -o beatmapset_novideo.osz
```

#### Download Single Beatmap (Individual Difficulty)
```
GET https://api.nerinyan.moe/b/{beatmap_id}
```

**Example:**
```bash
curl "https://api.nerinyan.moe/b/75" -o beatmap.osu
```

### Search Endpoints

```
GET https://api.nerinyan.moe/search
```

**Parameters:**
- `q` - Search query

**Example:**
```bash
curl "https://api.nerinyan.moe/search?q=Porter%20Robinson"
```

---

## 2. Mino / catboy.best

**Website:** https://catboy.best  
**Status:** Active, Free  
**Special Features:** Multiple regional servers, 100% osu!direct compatible  

### Regional Servers

- **Central (Germany):** https://catboy.best
- **US:** https://us.catboy.best  
- **Asia (Singapore):** https://sg.catboy.best

### Download Endpoints

#### Download Beatmapset
```
GET https://catboy.best/d/{beatmapset_id}
GET https://catboy.best/d/{beatmapset_id}n    (without video)
```

**Examples:**
```bash
# With video
curl "https://catboy.best/d/1" -o beatmapset.osz

# Without video
curl "https://catboy.best/d/1n" -o beatmapset_novideo.osz

# Using US server
curl "https://us.catboy.best/d/1" -o beatmapset.osz
```

#### Download Single Beatmap
```
GET https://catboy.best/b/{beatmap_id}
```

**Example:**
```bash
curl "https://catboy.best/b/75" -o beatmap.osu
```

### API Info Endpoints

#### Get Beatmapset Info (JSON)
```
GET https://catboy.best/api/s/{beatmapset_id}
```

**Example:**
```bash
curl "https://catboy.best/api/s/1"
```

**Response:** JSON with beatmapset metadata

#### Get Beatmap Info
```
GET https://catboy.best/api/b/{beatmap_id}
```

**Example:**
```bash
curl "https://catboy.best/api/b/75"
```

---

## 3. osu.direct (formerly Kitsu)

**Website:** https://osu.direct  
**Status:** Active, Free  
**Previous Name:** kitsu.moe  
**Special Features:** v1 and v2 API routes, used by osu!droid and Akatsuki  

### Download Endpoints

Based on the implementation patterns observed, osu.direct follows the Kitsu/CheeseGull API structure:

#### Download Beatmapset
```
GET https://osu.direct/d/{beatmapset_id}
GET https://osu.direct/d/{beatmapset_id}n     (without video)
```

**Examples:**
```bash
# With video
curl "https://osu.direct/d/1" -o beatmapset.osz

# Without video
curl "https://osu.direct/d/1n" -o beatmapset_novideo.osz
```

#### Alternative Download Endpoint (Legacy Compatibility)
```
GET https://osu.direct/api/d/{beatmapset_id}
```

### Search/Browse Endpoints

#### Search Beatmaps
```
GET https://osu.direct/api/search
```

**Query Parameters:**
- `query` - Search text
- `mode` - Game mode (0=std, 1=taiko, 2=catch, 3=mania)
- `status` - Ranked status
- `amount` - Results per page (default: 50)
- `offset` - Pagination offset

**Example:**
```bash
curl "https://osu.direct/api/search?query=night&mode=0&amount=10"
```

#### Get Beatmapset Info
```
GET https://osu.direct/api/s/{beatmapset_id}
```

**Example:**
```bash
curl "https://osu.direct/api/s/1"
```

#### Get Beatmap Info
```
GET https://osu.direct/api/b/{beatmap_id}
```

**Example:**
```bash
curl "https://osu.direct/api/b/75"
```

### Notes
- osu.direct maintains backward compatibility with Kitsu API
- Supports both v1 and v2 API routes
- Trusted by major projects (osu!droid, Akatsuki)

---

## 4. Sayobot (osu.sayobot.cn)

**Website:** https://osu.sayobot.cn  
**Download Base:** https://dl.sayobot.cn  
**Status:** Active, Free  
**Special Features:** Chinese mirror, global access  

### Download Endpoints

#### Download Beatmapset with Video
```
GET https://dl.sayobot.cn/beatmaps/download/full/{beatmapset_id}
```

**Example:**
```bash
curl "https://dl.sayobot.cn/beatmaps/download/full/1" -o beatmapset.osz
```

#### Download Beatmapset without Video
```
GET https://dl.sayobot.cn/beatmaps/download/novideo/{beatmapset_id}
```

**Example:**
```bash
curl "https://dl.sayobot.cn/beatmaps/download/novideo/1" -o beatmapset_novideo.osz
```

### Notes
- Single beatmap (.osu) downloads not documented

---

## Common API Patterns Across Mirrors

### Standard Endpoints

Most mirrors follow similar patterns:

```
/d/{id}         - Download beatmapset with video
/d/{id}n        - Download beatmapset without video
/b/{id}         - Download single beatmap (.osu file)
/api/s/{id}     - Get beatmapset info (JSON)
/api/b/{id}     - Get beatmap info (JSON)
/api/search     - Search beatmaps
```

### Response Formats

**Successful Download:**
- Content-Type: `application/octet-stream` or `application/x-osu-beatmap-archive`
- File extension: `.osz` for beatmapsets, `.osu` for single beatmaps

**API Info Responses:**
- Content-Type: `application/json`
- Returns beatmap metadata

---

## Complete Usage Examples

### Download Same Beatmapset from All Active Mirrors

```bash
# Nerinyan
curl "https://api.nerinyan.moe/d/1" -o beatmap_nerinyan.osz

# catboy.best (Mino)
curl "https://catboy.best/d/1" -o beatmap_catboy.osz

# osu.direct
curl "https://osu.direct/d/1" -o beatmap_osudirect.osz

# Sayobot
curl "https://dl.sayobot.cn/beatmaps/download/full/1" -o beatmap_sayobot.osz
```

### Download Without Video (Smaller Files)

```bash
# Nerinyan
curl "https://api.nerinyan.moe/d/1?nv=1" -o beatmap.osz

# catboy.best
curl "https://catboy.best/d/1n" -o beatmap.osz

# osu.direct  
curl "https://osu.direct/d/1n" -o beatmap.osz

# Sayobot
curl "https://dl.sayobot.cn/beatmaps/download/novideo/1" -o beatmap.osz
```

### Get Beatmapset Information (JSON)

```bash
# catboy.best
curl "https://catboy.best/api/s/1"

# osu.direct
curl "https://osu.direct/api/s/1"
```

---

## Best Practices

1. **Respect Rate Limits:** All mirrors are free but limited.
2. **Use Regional Servers:** catboy.best offers multiple regions
3. **Implement Fallbacks:** If one mirror is down, try another
4. **Cache Downloads:** Don't re-download the same beatmaps
5. **Set User-Agent:** Identify your application properly
6. **Handle Errors:** Implement retry logic with exponential backoff

### Recommended User-Agent Format
```
User-Agent: YourAppName/Version (contact@email.com)
```

---

## Troubleshooting

**404 Not Found:**
- Beatmap may not exist or was removed
- Try a different mirror

**Slow Downloads:**
- Try a different regional mirror (catboy.best)
- Use novideo option to reduce file size

**Connection Timeouts:**
- Mirror may be experiencing high load
- Try another mirror
- Implement retry logic

**Hash Mismatches:**
- Some old beatmaps have known hash issues
- Mainly affects beatmaps from 2007-2008
- This is a known issue with official osu! servers
