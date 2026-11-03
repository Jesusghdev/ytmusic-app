use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::webview::{Color, WebviewBuilder};
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Rect, WebviewUrl,
};

const BRIDGE_JS: &str = include_str!("bridge.js");

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UpcomingTrack {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub duration: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlayerState {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub artwork: String,
    #[serde(default)]
    pub video_id: String,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub position: f64,
    #[serde(default)]
    pub playing: bool,
    #[serde(default)]
    pub volume: f64,
    #[serde(default)]
    pub liked: bool,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub next_track: Option<UpcomingTrack>,
}

impl PlayerState {
    pub fn empty() -> Self {
        Self {
            volume: 1.0,
            ..Default::default()
        }
    }

    pub fn sanitize(&mut self) {
        self.title = clean_text(&self.title, 300);
        self.artist = clean_text(&self.artist, 300);
        self.album = clean_text(&self.album, 300);
        if let Some(next) = &mut self.next_track {
            next.title = clean_text(&next.title, 300);
            next.artist = clean_text(&next.artist, 300);
            next.album = clean_text(&next.album, 300);
            next.duration = if next.duration.is_finite() && next.duration > 0.0 {
                next.duration.min(86_400.0)
            } else {
                0.0
            };
            if next.title.is_empty() {
                self.next_track = None;
            }
        }

        if !(self.artwork.starts_with("https://") || self.artwork.starts_with("http://")) {
            self.artwork.clear();
        } else if self.artwork.len() > 2000 {
            self.artwork.truncate(2000);
        }

        self.video_id = self
            .video_id
            .chars()
            .filter(|character| {
                character.is_ascii_alphanumeric() || *character == '-' || *character == '_'
            })
            .take(32)
            .collect();

        self.duration = if self.duration.is_finite() && self.duration > 0.0 {
            self.duration.min(86_400.0)
        } else {
            0.0
        };

        self.position = if self.position.is_finite() && self.position > 0.0 {
            if self.duration > 0.0 {
                self.position.min(self.duration)
            } else {
                self.position
            }
        } else {
            0.0
        };

        self.volume = if self.volume.is_finite() {
            self.volume.clamp(0.0, 1.0)
        } else {
            1.0
        };
    }
}

fn clean_text(value: &str, max: usize) -> String {
    let text = value
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    text.chars().take(max).collect()
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Extrapola la posición real desde la última actualización recibida.
pub fn effective(state: &PlayerState) -> PlayerState {
    let mut result = state.clone();
    if result.playing && result.updated_at > 0 {
        let elapsed = now_ms().saturating_sub(result.updated_at) as f64 / 1000.0;
        result.position += elapsed;
        if result.duration > 0.0 {
            result.position = result.position.min(result.duration);
        }
    }
    result
}

fn bridge_script(port: u16) -> String {
    BRIDGE_JS.replace("__PORT__", &port.to_string())
}

/// Crea (o recoloca) YouTube Music incrustado dentro de la ventana principal.
/// Debe llamarse desde un comando async: en Windows, crear webviews desde el
/// hilo principal bloquea el bucle de eventos.
pub fn create_embedded(
    app: &AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    if app.get_webview("ytm").is_some() {
        return set_embedded_bounds(app, x, y, width, height);
    }

    let port = app.state::<crate::Shared>().port.load(Ordering::SeqCst);
    if port == 0 {
        return Err("El servidor interno no está disponible.".to_string());
    }

    let window = app
        .get_window("main")
        .ok_or_else(|| "No se encontró la ventana principal.".to_string())?;

    let url = "https://music.youtube.com/"
        .parse()
        .map_err(|_| "No se pudo construir la URL de YouTube Music.".to_string())?;

    let builder = WebviewBuilder::new("ytm", WebviewUrl::External(url))
        .initialization_script(bridge_script(port))
        .background_color(Color(10, 10, 12, 255));

    window
        .add_child(
            builder,
            LogicalPosition::new(x.max(0.0), y.max(0.0)),
            LogicalSize::new(width.max(1.0), height.max(1.0)),
        )
        .map_err(|error| format!("No se pudo incrustar YouTube Music: {error}"))?;

    Ok(())
}

/// Ajusta el rectángulo que ocupa YouTube Music dentro de la ventana.
/// Las coordenadas están en píxeles lógicos (los mismos de CSS).
pub fn set_embedded_bounds(
    app: &AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let Some(webview) = app.get_webview("ytm") else {
        return Ok(());
    };

    if width < 1.0 || height < 1.0 {
        let _ = webview.hide();
        return Ok(());
    }

    let _ = webview.show();

    webview
        .set_bounds(Rect {
            position: LogicalPosition::new(x.max(0.0), y.max(0.0)).into(),
            size: LogicalSize::new(width.max(1.0), height.max(1.0)).into(),
        })
        .map_err(|error| error.to_string())
}

/// Oculta o muestra el webview integrado (por ejemplo, en el modo pantalla grande).
pub fn set_visible(app: &AppHandle, visible: bool) -> Result<(), String> {
    let Some(webview) = app.get_webview("ytm") else {
        return Ok(());
    };

    if visible {
        webview.show().map_err(|error| error.to_string())
    } else {
        // No usamos hide(): una página oculta frena sus timers y las
        // actualizaciones de tiempo dejan de llegar. La movemos fuera de la
        // ventana conservando su tamaño, para que el reproductor siga activo
        // y la posición se mantenga sincronizada.
        let _ = webview.show();
        webview
            .set_position(LogicalPosition::new(20_000.0, 20_000.0))
            .map_err(|error| error.to_string())
    }
}

/// Fuerza un envío inmediato del estado desde el reproductor incrustado.
/// Se usa en pantalla grande para no depender del temporizador de la página.
pub fn nudge_state(app: &AppHandle) -> Result<(), String> {
    let Some(webview) = app.get_webview("ytm") else {
        return Ok(());
    };

    webview
        .eval("window.__ytm && window.__ytm.sendNow && window.__ytm.sendNow()")
        .map_err(|error| error.to_string())
}

pub fn eval_command(app: &AppHandle, action: &str, value: Option<f64>) -> Result<(), String> {    let Some(webview) = app.get_webview("ytm") else {
        return Err("YouTube Music todavía no está listo.".to_string());
    };

    let js = match action {
        "play" => "window.__ytm && window.__ytm.play()".to_string(),
        "pause" => "window.__ytm && window.__ytm.pause()".to_string(),
        "toggle" => "window.__ytm && window.__ytm.toggle()".to_string(),
        "next" => "window.__ytm && window.__ytm.next()".to_string(),
        "prev" => "window.__ytm && window.__ytm.prev()".to_string(),
        "like" => "window.__ytm && window.__ytm.like()".to_string(),
        "seek" => {
            let seconds = value
                .filter(|value| value.is_finite())
                .ok_or_else(|| "Posición inválida.".to_string())?;
            format!("window.__ytm && window.__ytm.seek({seconds})")
        }
        "volume" => {
            let volume = value
                .filter(|value| value.is_finite())
                .ok_or_else(|| "Volumen inválido.".to_string())?;
            format!("window.__ytm && window.__ytm.volume({volume})")
        }
        other => return Err(format!("Acción desconocida: {other}")),
    };

    webview.eval(js).map_err(|error| error.to_string())
}
