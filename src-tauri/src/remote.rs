use crate::{player, Shared};
use serde::Deserialize;
use std::io::Read;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const REMOTE_HTML: &str = include_str!("remote.html");

pub fn spawn(app: AppHandle) {
    let Some((server, port)) = bind() else {
        eprintln!("[remote] no se pudo abrir ningún puerto para el control remoto");
        return;
    };

    if let Some(shared) = app.try_state::<Shared>() {
        shared.port.store(port, Ordering::SeqCst);
    }

    println!("[remote] control remoto escuchando en el puerto {port}");

    let result = std::thread::Builder::new()
        .name("ytm-remote".to_string())
        .spawn(move || {
            for request in server.incoming_requests() {
                handle(&app, request);
            }
        });

    if let Err(error) = result {
        eprintln!("[remote] no se pudo iniciar el hilo del servidor: {error}");
    }
}

fn bind() -> Option<(Server, u16)> {
    for port in 47_821u16..47_861u16 {
        if let Ok(server) = Server::http(("0.0.0.0", port)) {
            return Some((server, port));
        }
    }
    None
}

fn handle(app: &AppHandle, mut request: Request) {
    let method = request.method().clone();
    let path = request
        .url()
        .split('?')
        .next()
        .unwrap_or("")
        .to_string();

    let mut body = String::new();
    let _ = request
        .as_reader()
        .take(1_000_000)
        .read_to_string(&mut body);

    let (status, content_type, payload) = match (method, path.as_str()) {
        (Method::Options, _) => (204, "text/plain; charset=utf-8", String::new()),
        (Method::Get, "/") => (
            200,
            "text/html; charset=utf-8",
            REMOTE_HTML.to_string(),
        ),
        (Method::Get, "/health") => (200, "text/plain; charset=utf-8", "ok".to_string()),
        (Method::Get, "/api/state") => {
            let state = app
                .try_state::<Shared>()
                .and_then(|shared| {
                    shared
                        .player
                        .lock()
                        .ok()
                        .map(|guard| player::effective(&guard))
                })
                .unwrap_or_else(player::PlayerState::empty);

            (
                200,
                "application/json; charset=utf-8",
                serde_json::to_string(&state).unwrap_or_else(|_| "{}".to_string()),
            )
        }
        (Method::Post, "/api/state") => match serde_json::from_str::<player::PlayerState>(&body) {
            Ok(state) => {
                crate::ingest_state(app, state);
                (204, "text/plain; charset=utf-8", String::new())
            }
            Err(error) => (
                400,
                "application/json; charset=utf-8",
                json_error(&format!("Estado inválido: {error}")),
            ),
        },
        (Method::Post, "/api/command") => match serde_json::from_str::<CommandBody>(&body) {
            Ok(command) => match player::eval_command(app, &command.action, command.value) {
                Ok(()) => (
                    200,
                    "application/json; charset=utf-8",
                    "{\"ok\":true}".to_string(),
                ),
                Err(error) => (
                    400,
                    "application/json; charset=utf-8",
                    json_error(&error),
                ),
            },
            Err(error) => (
                400,
                "application/json; charset=utf-8",
                json_error(&format!("Comando inválido: {error}")),
            ),
        },
        _ => (404, "text/plain; charset=utf-8", "No encontrado".to_string()),
    };

    let data = payload.into_bytes();
    let mut response = Response::from_data(data).with_status_code(StatusCode(status));

    for (name, value) in [
        ("Access-Control-Allow-Origin", "*"),
        ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
        ("Access-Control-Allow-Headers", "Content-Type"),
        ("Access-Control-Allow-Private-Network", "true"),
        ("Access-Control-Max-Age", "86400"),
        ("Cache-Control", "no-store"),
        ("X-Content-Type-Options", "nosniff"),
    ] {
        if let Ok(header) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            response.add_header(header);
        }
    }

    if let Ok(header) = Header::from_bytes("Content-Type".as_bytes(), content_type.as_bytes()) {
        response.add_header(header);
    }

    let _ = request.respond(response);
}

#[derive(Debug, Deserialize)]
struct CommandBody {
    action: String,
    value: Option<f64>,
}

fn json_error(message: &str) -> String {
    serde_json::json!({ "ok": false, "error": message }).to_string()
}

pub fn lan_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip())
}

pub fn remote_url(port: u16) -> String {
    let host = lan_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    format!("http://{host}:{port}/")
}
