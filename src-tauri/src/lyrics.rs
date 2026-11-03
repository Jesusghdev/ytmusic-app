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
    #[serde(rename = "trackName")]
    track_name: Option<String>,
    #[serde(rename = "artistName")]
    artist_name: Option<String>,
    duration: Option<f64>,
    instrumental: Option<bool>,
    #[serde(rename = "plainLyrics")]
    plain_lyrics: Option<String>,
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("ytmusic-desktop/0.2 (app de escritorio personal)")
        .timeout(Duration::from_secs(5))
        .connect_timeout(Duration::from_secs(3))
        .build()
        .map_err(|error| error.to_string())
}

pub async fn fetch(
    title: &str,
    artist: &str,
    album: &str,
    duration: f64,
) -> Result<Lyrics, String> {
    let title = title.trim();
    let artist = artist.trim();
    let album = album.trim();

    if title.is_empty() {
        return Ok(Lyrics::default());
    }

    let http = client()?;

    // Las fuentes se consultan a la vez. Antes se encadenaban: una canción
    // sin letra podía tardar más de 30 s entre consultas y reintentos.
    let normalized_title = normalize_query(title);
    let normalized_artist = normalize_query(artist);
    let (exact, exact_without_duration, candidates, plain) = tokio::join!(
        lrclib_get(&http, title, artist, album, duration),
        lrclib_get(&http, title, artist, "", 0.0),
        lrclib_search(&http, title, artist),
        fetch_plain_fallback(&http, title, artist),
    );

    for track in [exact, exact_without_duration].into_iter().flatten() {
        if matches_request(&track, title, artist) {
            let result = to_result(track);
            if !result.lines.is_empty() || result.instrumental {
                return Ok(result);
            }
        }
    }

    if let Some(track) = pick_best(candidates, title, artist, duration) {
        let result = to_result(track);
        if !result.lines.is_empty() || result.instrumental {
            return Ok(result);
        }
    }

    // Algunas canciones aparecen en LRCLIB solo con una consulta libre y sin
    // puntuación. Se prueban variantes normalizadas únicamente como fallback
    // para no golpear innecesariamente el límite de la API.
    let (normalized_candidates, free, title_only) = tokio::join!(
        lrclib_search_free(&http, &normalized_title, &normalized_artist),
        lrclib_search_free(&http, title, artist),
        lrclib_search_free(&http, &normalized_title, ""),
    );
    let mut fallback_candidates = normalized_candidates;
    fallback_candidates.extend(free);
    fallback_candidates.extend(title_only);
    if let Some(track) = pick_best(fallback_candidates, title, artist, duration) {
        let result = to_result(track);
        if !result.lines.is_empty() || result.instrumental {
            return Ok(result);
        }
    }

    if let Some(plain) = plain {
        return Ok(plain);
    }

    Ok(Lyrics {
        message: "No encontramos la letra para esta canción.".to_string(),
        ..Default::default()
    })
}

async fn lrclib_get(
    http: &reqwest::Client,
    title: &str,
    artist: &str,
    album: &str,
    duration: f64,
) -> Option<LrcLibTrack> {
    let mut query: Vec<(&str, String)> = vec![
        ("track_name", title.to_string()),
        ("artist_name", artist.to_string()),
    ];
    if !album.is_empty() {
        query.push(("album_name", album.to_string()));
    }
    if duration.is_finite() && duration > 0.0 {
        query.push(("duration", duration.round().max(1.0).to_string()));
    }

    let response = http
        .get("https://lrclib.net/api/get")
        .query(&query)
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    response.json::<LrcLibTrack>().await.ok()
}

async fn lrclib_search(
    http: &reqwest::Client,
    title: &str,
    artist: &str,
) -> Vec<LrcLibTrack> {
    let artist_name = if artist.is_empty() { title } else { artist };

    let response = http
        .get("https://lrclib.net/api/search")
        .query(&[("track_name", title), ("artist_name", artist_name)])
        .send()
        .await;

    match response {
        Ok(response) if response.status().is_success() => {
            response.json::<Vec<LrcLibTrack>>().await.unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

async fn lrclib_search_free(
    http: &reqwest::Client,
    title: &str,
    artist: &str,
) -> Vec<LrcLibTrack> {
    let response = http
        .get("https://lrclib.net/api/search")
        .query(&[("q", format!("{title} {artist}"))])
        .send()
        .await;

    match response {
        Ok(response) if response.status().is_success() => {
            response.json::<Vec<LrcLibTrack>>().await.unwrap_or_default()
        }
        _ => Vec::new(),
    }
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

fn normalize_query(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character.is_whitespace() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalized_equal(left: &str, right: &str) -> bool {
    normalize_query(left).eq_ignore_ascii_case(&normalize_query(right))
}

fn artist_tokens(value: &str) -> Vec<String> {
    normalize_query(value)
        .split_whitespace()
        .filter(|token| *token != "y" && *token != "and")
        .map(str::to_ascii_lowercase)
        .collect()
}

fn artist_overlap(left: &str, right: &str) -> usize {
    let left = artist_tokens(left);
    let right = artist_tokens(right);
    left.iter()
        .filter(|token| right.iter().any(|candidate| candidate == *token))
        .count()
}

fn matches_request(track: &LrcLibTrack, title: &str, artist: &str) -> bool {
    let title_matches = track
        .track_name
        .as_deref()
        .map(|value| normalized_equal(value, title))
        .unwrap_or(false);
    let requested_artists = artist_tokens(artist);
    let artist_matches = requested_artists.is_empty()
        || track.artist_name.as_deref().map_or(false, |value| {
            normalized_equal(value, artist)
                || artist_overlap(value, artist)
                    >= requested_artists.len().div_ceil(2).max(1)
        });
    title_matches && artist_matches
}

fn score(track: &LrcLibTrack, title: &str, artist: &str, duration: f64) -> f64 {
    let mut score = 0.0;

    if track
        .track_name
        .as_deref()
        .map(|value| normalized_equal(value, title))
        .unwrap_or(false)
    {
        score -= 2_000.0;
    }
    if let Some(value) = track.artist_name.as_deref() {
        if normalized_equal(value, artist) {
            score -= 1_000.0;
        } else {
            score -= (artist_overlap(value, artist) as f64) * 250.0;
        }
    }

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

fn pick_best(
    mut tracks: Vec<LrcLibTrack>,
    title: &str,
    artist: &str,
    duration: f64,
) -> Option<LrcLibTrack> {
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
        score(left, title, artist, duration)
            .partial_cmp(&score(right, title, artist, duration))
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

    #[test]
    fn matches_collaborating_artist_credits() {
        let track = LrcLibTrack {
            track_name: Some("Estar Así Siempre".to_string()),
            artist_name: Some("Nsqk, Julieta Venegas, mosca".to_string()),
            duration: Some(249.0),
            instrumental: Some(false),
            plain_lyrics: None,
            synced_lyrics: Some("[00:01.00]Hola".to_string()),
        };

        assert!(matches_request(
            &track,
            "# Estar Así Siempre",
            "Nsqk y Julieta Venegas"
        ));
    }
}
