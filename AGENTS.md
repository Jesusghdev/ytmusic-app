# AGENTS.md — Guía para agentes de IA

Este archivo describe el proyecto para que cualquier agente (o persona) lo entienda y contribuya con rapidez, sin leer todo el código. Léelo antes de hacer cambios.

## Qué es este proyecto

**YouTube Music Desktop** es un cliente de escritorio no oficial de YouTube Music para Windows. Combina la web oficial (`music.youtube.com`) incrustada en un WebView2 con una capa nativa hecha en Tauri que añade:

- **Letras sincronizadas** (resaltado de la línea actual, clic para saltar, vista inmersiva).
- **Pantalla grande** tipo karaoke (carátula difuminada de fondo, controles y letra).
- **Control remoto** desde el celular en la misma red (QR + servidor HTTP local).
- **Barra de título propia**, atajos de teclado y sesión persistente.

No es una reimplementación del reproductor: la reproducción, la búsqueda y la biblioteca ocurren dentro del WebView2 de YouTube Music; la app solo lee el estado y añade funcionalidad por encima.

## Stack

| Capa | Tecnología |
| --- | --- |
| Shell de escritorio | Tauri 2 (Rust) |
| Frontend | Vue 3 (`<script setup>`) + TypeScript + Vite |
| WebView | Microsoft Edge WebView2 (Windows) |
| Letras | LRCLIB (sincronizadas) + Lyrics.ovh (texto de respaldo) |
| Control remoto | Servidor HTTP local (`tiny_http`) + token persistente |

## Cómo funciona (flujo de datos)

El estado del reproductor viaja en esta cadena:

```
YouTube Music (WebView2)
  └─ bridge.js  ← se inyecta vía initialization_script
       └─ POST /api/state  →  servidor local (remote.rs)  [fallback HTTP]
            └─  o bien invoke "plugin:event|emit"  [IPC directo, preferido]
                 └─ Rust lib.rs → ingest_state()
                      └─ evento Tauri "player-state"
                           └─ Vue App.vue → applyPlayer() → UI
```

Detalles importantes:

1. **`bridge.js`** (en `src-tauri/src/bridge.js`) se inyecta en la página de YouTube Music. Lee el estado con `navigator.mediaSession` + el elemento `<video>` + la barra oficial (`ytmusic-player-bar`), y expone `window.__ytm` para recibir comandos (`play`, `pause`, `next`, `seek`, `volume`, `like`…).
2. **Doble canal de envío**: primero intenta IPC (`__TAURI_INTERNALS__.invoke`); si falla, cae a HTTP con `fetch` en modo `no-cors` + `text/plain` (evita preflight). El cuerpo se envía igual.
3. **El puerto del servidor local** lo abre `remote.rs` en el rango `47821..47861` y se guarda en `Shared.port`. Ese mismo servidor sirve `/api/state` (ingesta del bridge), la web del control remoto y el estado del reproductor.
4. **Sincronización de posición**: la app extrapola la posición real con `player::effective()` (usa `updated_at` + reloj local) y en la UI con `anchorPosition`/`anchorAt`. No confía ciegamente en cada lectura del puente.

## Estructura de archivos

```
src/                         # Frontend Vue
  App.vue                    # Toda la lógica de UI y estado (archivo grande)
  components/LyricsView.vue  # Vista de letras (panel + pantalla grande)
  types.ts                   # Tipos compartidos del frontend
src-tauri/
  src/
    main.rs                  # Punto de entrada binario
    lib.rs                   # Setup de Tauri, Shared, comandos y eventos
    player.rs                # PlayerState, sanitización, webview embebido, comandos
    bridge.js                # Script inyectado en YouTube Music
    lyrics.rs                # Búsqueda y parseo de letras (LRCLIB + Lyrics.ovh)
    remote.rs                # Servidor HTTP local + token + control remoto
    remote.html              # UI del control remoto (servida por el servidor)
  tauri.conf.json            # Config de Tauri (ventana, CSP, bundle)
  Cargo.toml                 # Dependencias Rust
ROADMAP.md                   # Roadmap privado (está en .gitignore)
```

### Mapa de responsabilidades

- **`lib.rs`**: define `Shared` (estado mutex + puerto + token), registra los comandos de Tauri y el listener `bridge-state`, y emite `player-state` a la UI. `ingest_state()` sanitiza y reenvía.
- **`player.rs`**: `PlayerState` y `sanitize()` (limpieza de texto, límites de longitud, clamp de volumen/duración/posición, validación de URL de carátula y de `video_id`). Crea/recoloca/oculta el webview `ytm`. `eval_command()` traduce acciones a JS.
- **`lyrics.rs`**: `fetch()` consulta LRCLIB (exacto, search y free search) y Lyrics.ovh a la vez, puntúa candidatos y parsea LRC. Incluye tests unitarios (`cargo test`).
- **`remote.rs`**: abre el puerto, genera/carga el token persistente, y enruta `GET /` (web remota), `GET/POST /api/state`, `POST /api/command`, `GET /health`.
- **`App.vue`**: orquesta el estado del reproductor, la posición extrapolada, la caché de letras (`localStorage`), el modo pantalla grande y el panel lateral. Contiene bastante lógica sutil de sincronización (ver "Consideraciones").

## Comandos de Tauri (invocados desde la UI)

| Comando | Descripción |
| --- | --- |
| `get_player_state` | Devuelve el estado efectivo del reproductor |
| `init_ytm` | Crea el webview `ytm` con la URL de YouTube Music |
| `set_ytm_bounds` | Ajusta el rectángulo del webview embebido |
| `set_ytm_visible` | Muestra/oculta el webview (en pantalla grande lo mueve fuera de la ventana, no usa `hide()`) |
| `ytm_command` | Ejecuta una acción (`play`, `pause`, `toggle`, `next`, `prev`, `like`, `seek`, `volume`) |
| `ytm_nudge` | Fuerza un envío inmediato de estado (pantalla grande) |
| `get_lyrics` | Busca la letra (LRCLIB + Lyrics.ovh) |
| `remote_info` | Devuelve la URL con token y el puerto del control remoto |

## API del control remoto (servidor local)

- `GET /` → HTML del control remoto (`remote.html`).
- `GET /health` → `ok` (sin autenticación).
- `GET /api/state` → estado actual del reproductor (JSON).
- `POST /api/state` → ingesta manual de estado.
- `POST /api/command` → `{ "action": "...", "value": 123 }`.

Autenticación: token en query (`?token=…` o `?remote_token=…`) o cabecera `X-Remote-Token`. El token es aleatorio (64 hex), se persiste en `app_config_dir/remote-token` y se genera una sola vez.

## Atajos de teclado (App.vue)

- `Espacio` reproducir/pausar · `←`/`→` retroceder/avanzar 10 s · `N`/`P` siguiente/anterior · `L` letra · `R` remoto · `F` pantalla grande · `Esc` salir de pantalla grande.

## Consideraciones y convenciones importantes

Estos son detalles que NO deben romperse al modificar código:

1. **No registrar títulos de canciones en la consola.** En `lib.rs` hay un comentario explícito: no se loguea el título para no dejar rastro del historial de reproducción. Mantén ese criterio de privacidad.
2. **Privacidad**: nunca incluir contraseñas, cookies, tokens o claves de API en el repo. Las búsquedas de letras solo envían título/artista/álbum/duración a LRCLIB y Lyrics.ovh.
3. **`sanitize()`** valida todo lo que llega del bridge (el contenido viene de una web externa). Cualquier campo nuevo que se añada a `PlayerState` debe sanearse ahí.
4. **El webview nunca se oculta con `hide()`** en pantalla grande: una página oculta frena sus timers y deja de actualizar la posición. Se mueve fuera de la ventana (`set_position` a `20_000, 20_000`).
5. **Sincronización de posición** en `App.vue` tiene varias ventanas temporales (`holdUntil`, `settleUntil`, `seekGuardUntil`, `stableDuration`) para evitar saltos/parpadeos durante el cambio automático de canción. Toca con cuidado y prueba con cambios de pista reales.
6. **El bridge es frágil**: depende de selectores del DOM de YouTube Music (`#movie_player video`, `ytmusic-player-bar`, etc.). Los cambios de YouTube pueden romperlo; los selectores tienen múltiples variantes como respaldo.
7. **Letras**: `lyrics.rs` consulta varias fuentes en paralelo y puntúa candidatos (coincidencia de título/artista/duración). Tiene tests con `cargo test`. LRCLIB requiere el header `Lrclib-App-Id` (ver `LRCLIB_APP_ID`).
8. **Idioma**: código y comentarios en español; mensajes de UI en español.

## Comandos de desarrollo

```bash
npm install          # instalar dependencias frontend
npm run tauri dev    # desarrollo (hot reload)
npm run tauri build  # generar instalador/portable
cargo test           # tests de lyrics.rs (desde src-tauri)
```

Requisitos: Node.js 20+, Rust estable y WebView2.

## Roadmap

El roadmap detallado está en `ROADMAP.md` (incluido en `.gitignore`, por lo que **no se publica** en GitHub). Pendientes actuales:

- [ ] Fallback experimental de letras leyendo la pestaña de YouTube Music (scraping). Solo uso personal; revisar términos de uso y fragilidad.
- [ ] Importación manual de letra o archivo `.lrc`.
- [ ] PIN/token para autenticar el control remoto.
- [ ] CSP para la ventana principal (hoy `security.csp` es `null` en `tauri.conf.json`).
- [ ] Revisar privacidad del historial de Git (email de los commits).
- [ ] Evaluar más fuentes abiertas de letras además de LRCLIB y Lyrics.ovh.

## Contacto / repo

- Repositorio: `github.com/Jesusghdev/ytmusic-app`.
- Licencia: MIT (las marcas de YouTube Music pertenecen a Google).
