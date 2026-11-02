mod lyrics;
mod player;
mod remote;

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Listener, Manager, State, WindowEvent};

pub struct Shared {
    player: Mutex<player::PlayerState>,
    port: AtomicU16,
}

impl Shared {
    fn new() -> Self {
        Self {
            player: Mutex::new(player::PlayerState::empty()),
            port: AtomicU16::new(0),
        }
    }
}

/// Guarda el estado que llega desde el puente de YouTube Music (IPC o HTTP).
pub fn ingest_state(app: &AppHandle, mut state: player::PlayerState) {
    state.sanitize();
    state.updated_at = player::now_ms();

    if let Some(shared) = app.try_state::<Shared>() {
        if let Ok(mut guard) = shared.player.lock() {
            if guard.updated_at == 0 {
                println!("[bridge] conectado con YouTube Music");
            }
            *guard = state.clone();
        }
    }

    // No registramos el título de la canción para no dejar rastro del historial
    // de reproducción en la consola.
    let _ = app.emit("player-state", &state);
}

#[tauri::command]
fn get_player_state(shared: State<'_, Shared>) -> player::PlayerState {
    shared
        .player
        .lock()
        .map(|state| player::effective(&state))
        .unwrap_or_else(|_| player::PlayerState::empty())
}

#[tauri::command]
async fn init_ytm(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    player::create_embedded(&app, x, y, width, height)
}

#[tauri::command]
fn set_ytm_bounds(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    player::set_embedded_bounds(&app, x, y, width, height)
}

#[tauri::command]
fn set_ytm_visible(app: AppHandle, visible: bool) -> Result<(), String> {
    player::set_visible(&app, visible)
}

#[tauri::command]
fn ytm_command(app: AppHandle, action: String, value: Option<f64>) -> Result<(), String> {
    player::eval_command(&app, &action, value)
}

#[tauri::command]
fn ytm_nudge(app: AppHandle) -> Result<(), String> {
    player::nudge_state(&app)
}

#[tauri::command]
async fn get_lyrics(
    title: String,
    artist: String,
    album: String,
    duration: f64,
) -> Result<lyrics::Lyrics, String> {
    lyrics::fetch(&title, &artist, &album, duration).await
}

#[tauri::command]
fn remote_info(app: AppHandle) -> Result<serde_json::Value, String> {
    let port = app.state::<Shared>().port.load(Ordering::SeqCst);
    if port == 0 {
        return Err(
            "No se pudo iniciar el servidor de control remoto. Reinicia la aplicación."
                .to_string(),
        );
    }

    Ok(serde_json::json!({
        "url": remote::remote_url(port),
        "port": port,
    }))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Shared::new())
        .setup(|app| {
            remote::spawn(app.handle().clone());

            let bridge_handle = app.handle().clone();
            app.listen("bridge-state", move |event| {
                match serde_json::from_str::<player::PlayerState>(event.payload()) {
                    Ok(state) => ingest_state(&bridge_handle, state),
                    Err(error) => eprintln!("[bridge] estado inválido: {error}"),
                }
            });

            if let Some(main) = app.get_webview_window("main") {
                let handle = app.handle().clone();
                main.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { .. } = event {
                        handle.exit(0);
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_player_state,
            init_ytm,
            set_ytm_bounds,
            set_ytm_visible,
            ytm_command,
            ytm_nudge,
            get_lyrics,
            remote_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
