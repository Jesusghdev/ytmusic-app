use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Debug, Serialize)]
pub struct LyricLine {
    pub time: f64,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
    pub synced: bool,
    pub instrumental: bool,
    pub source: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize)]
struct LrcLibTrack {
    duration: Option<f64>,
    instrumental: Option<bool>,
    #[serde(rename = "plainLyrics")]
    plain_lyrics: Option<String>,
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("ytmusic-desktop/0.1 (app de escritorio personal)")
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|error| error.to_string())
}

pub async fn fetch(title: &str, artist: &str, album: &str, duration: f64) -> Result<Lyrics, String> {
    let title = title.trim();
    let artist = artist.trim();
    let album = album.trim();

    if title.is_empty() {
        return Ok(Lyrics::default());
    }

    let http = client()?;
    let duration_seconds = if duration.is_finite() && duration > 0.0 {
        Some(duration.round().max(1.0).to_string())
    } else {
        None
    };

    let mut query: Vec<(&str, String)> = vec![
        ("track_name", title.to_string()),
        ("artist_name", artist.to_string()),
    ];
    if !album.is_empty() {
        query.push(("album_name", album.to_string()));
    }
    if let Some(seconds) = &duration_seconds {
        query.push(("duration", seconds.clone()));
    }

    let response = http
        .get("https://lrclib.net/api/get")
        .query(&query)
        .send()
        .await
        .map_err(|error| format!("No se pudo consultar LRCLIB: {error}"))?;

    if response.status().is_success() {
        if let Ok(track) = response.json::<LrcLibTrack>().await {
            let result = to_result(track);
            if !result.lines.is_empty() || result.instrumental {
                return Ok(result);
            }
        }
    }

    // Segundo intento: búsqueda flexible.
    let mut search_query: Vec<(&str, String)> = vec![
        ("track_name", title.to_string()),
        ("artist_name", artist.to_string()),
    ];
    if search_query[1].1.is_empty() {
        search_query[1].1 = title.to_string();
    }

    let searched = http
        .get("https://lrclib.net/api/search")
        .query(&search_query)
        .send()
        .await
        .map_err(|error| format!("No se pudo buscar la letra: {error}"))?;

    let mut candidates = if searched.status().is_success() {
        searched.json::<Vec<LrcLibTrack>>().await.unwrap_or_default()
    } else {
        Vec::new()
    };

    if candidates.is_empty() {
        let fallback = http
            .get("https://lrclib.net/api/search")
            .query(&[("q", format!("{title} {artist}"))])
            .send()
            .await
            .map_err(|error| format!("No se pudo buscar la letra: {error}"))?;
        if fallback.status().is_success() {
            candidates = fallback.json::<Vec<LrcLibTrack>>().await.unwrap_or_default();
        }
    }

    if let Some(track) = pick_best(candidates, duration) {
        let result = to_result(track);
        if !result.lines.is_empty() || result.instrumental {
            return Ok(result);
        }
    }

    if let Some(plain) = fetch_plain_fallback(&http, title, artist).await {
        return Ok(plain);
    }

    Ok(Lyrics {
        message: "No encontramos la letra para esta canción.".to_string(),
        ..Default::default()
    })
}

/// Último recurso: letra sin sincronizar desde la API pública de Lyrics.ovh.
async fn fetch_plain_fallback(
    http: &reqwest::Client,
    title: &str,
    artist: &str,
) -> Option<Lyrics> {
    let artist = if artist.trim().is_empty() {
        title
    } else {
        artist
    };

    let mut url = reqwest::Url::parse("https://api.lyrics.ovh/v1/").ok()?;
    {
        let mut segments = url.path_segments_mut().ok()?;
        segments.push(artist);
        segments.push(title);
    }

    #[derive(Deserialize)]
    struct OvhLyrics {
        lyrics: Option<String>,
        error: Option<String>,
    }

    let response = http.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }

    let data = response.json::<OvhLyrics>().await.ok()?;
    if data.error.is_some() {
        return None;
    }

    let text = data.lyrics.unwrap_or_default();
    let lines = text
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .map(|text| LyricLine { time: -1.0, text })
        .collect::<Vec<_>>();

    if lines.is_empty() {
        return None;
    }

    Some(Lyrics {
        lines,
        synced: false,
        instrumental: false,
        source: "Lyrics.ovh".to_string(),
        message: String::new(),
    })
}

fn score(track: &LrcLibTrack, duration: f64) -> f64 {
    let mut score = 0.0;

    if track
        .synced_lyrics
        .as_deref()
        .map(|text| !text.trim().is_empty())
        .unwrap_or(false)
    {
        score -= 10_000.0;
    }

    if duration > 0.0 {
        match track.duration {
            Some(value) => score += (value - duration).abs(),
            None => score += 60.0,
        }
    }

    if track
        .plain_lyrics
        .as_deref()
        .map(|text| !text.trim().is_empty())
        .unwrap_or(false)
        || track.instrumental == Some(true)
    {
        score -= 10.0;
    }

    score
}

fn pick_best(mut tracks: Vec<LrcLibTrack>, duration: f64) -> Option<LrcLibTrack> {
    tracks.retain(|track| {
        track.instrumental == Some(true)
            || track
                .synced_lyrics
                .as_deref()
                .map(|text| !text.trim().is_empty())
                .unwrap_or(false)
            || track
                .plain_lyrics
                .as_deref()
                .map(|text| !text.trim().is_empty())
                .unwrap_or(false)
    });

    tracks.into_iter().min_by(|left, right| {
        score(left, duration)
            .partial_cmp(&score(right, duration))
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

fn to_result(track: LrcLibTrack) -> Lyrics {
    let instrumental = track.instrumental.unwrap_or(false);
    let synced_raw = track.synced_lyrics.unwrap_or_default();
    let plain_raw = track.plain_lyrics.unwrap_or_default();

    let synced_lines = parse_lrc(&synced_raw);
    if !synced_lines.is_empty() {
        return Lyrics {
            lines: synced_lines,
            synced: true,
            instrumental,
            source: "LRCLIB".to_string(),
            message: String::new(),
        };
    }

    let source_text = if !plain_raw.trim().is_empty() {
        plain_raw
    } else {
        synced_raw
    };

    let lines = source_text
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .map(|text| LyricLine { time: -1.0, text })
        .collect::<Vec<_>>();

    Lyrics {
        lines,
        synced: false,
        instrumental,
        source: if source_text.trim().is_empty() {
            String::new()
        } else {
            "LRCLIB".to_string()
        },
        message: String::new(),
    }
}

fn parse_lrc(input: &str) -> Vec<LyricLine> {
    if input.trim().is_empty() {
        return Vec::new();
    }

    let offset_seconds = input
        .lines()
        .find_map(|line| {
            let value = line.trim().strip_prefix("[offset:")?;
            let value = value.trim_end_matches(']').trim();
            value.parse::<f64>().ok()
        })
        .unwrap_or(0.0)
        / 1000.0;

    let mut lines = Vec::new();

    for raw_line in input.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        let mut rest = line;
        let mut timestamps = Vec::new();

        while let Some(after_bracket) = rest.strip_prefix('[') {
            let Some(end) = after_bracket.find(']') else {
                break;
            };

            let tag = &after_bracket[..end];
            match parse_timestamp(tag) {
                Some(seconds) => {
                    timestamps.push((seconds - offset_seconds).max(0.0));
                    rest = &after_bracket[end + 1..];
                }
                None => break,
            }
        }

        if timestamps.is_empty() {
            continue;
        }

        let text = rest.trim().to_string();

        for time in timestamps {
            lines.push(LyricLine {
                time,
                text: text.clone(),
            });
        }
    }

    lines.sort_by(|left, right| {
        left.time
            .partial_cmp(&right.time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    lines.dedup_by(|left, right| (left.time - right.time).abs() < 0.01 && left.text == right.text);
    lines
}

fn parse_timestamp(tag: &str) -> Option<f64> {
    let parts = tag.split(':').collect::<Vec<_>>();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }

    let mut total = 0.0;
    for (index, part) in parts.iter().enumerate() {
        let normalized = part.replace(',', ".");
        let value = normalized.parse::<f64>().ok()?;
        if value < 0.0 {
            return None;
        }

        let is_seconds = index == parts.len() - 1;
        total = if is_seconds {
            total * 60.0 + value
        } else {
            total * 60.0 + value.floor()
        };
    }

    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_timestamps() {
        let lines = parse_lrc("[00:12.50][00:45.00] Hola\n[01:02.25] Mundo");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "Hola");
        assert!((lines[0].time - 12.5).abs() < 0.001);
        assert!((lines[2].time - 62.25).abs() < 0.001);
    }

    #[test]
    fn ignores_metadata() {
        let lines = parse_lrc("[ar:Queen]\n[00:01.00]Hola");
        assert_eq!(lines.len(), 1);
    }
}
