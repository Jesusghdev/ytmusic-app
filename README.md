# YouTube Music Desktop

Aplicación de escritorio para Windows construida con **Tauri 2 + Vue 3 + TypeScript**. YouTube Music está **integrado dentro de la misma ventana**, junto a la letra sincronizada y el control remoto:

- **Tu cuenta de Google**: inicia sesión una vez y la sesión queda guardada en el equipo.
- **Navegación integrada**: busca canciones, abre tu biblioteca, álbumes y playlists sin abrir otra ventana.
- **Letra sincronizada** estilo Spotify: línea activa iluminada, resaltado progresivo y salto al hacer clic en una línea.
- **Pantalla grande**: una vista a pantalla completa con la letra a la izquierda, la carátula y los datos del tema a la derecha, y controles de reproducción.
- **Control remoto desde el celular**: un servidor HTTP local sirve una página móvil con play/pausa, anterior/siguiente, barra de progreso y volumen. Se accede con el código QR que muestra la app.

## Requisitos

- Windows 10/11 con WebView2 (viene instalado en Windows 11 y en Windows 10 actualizado).
- [Node.js](https://nodejs.org/) 20 o superior.
- [Rust](https://www.rust-lang.org/tools/install) (toolchain estable de MSVC).

## Desarrollo

```bash
npm install
npm run tauri dev
```

## Uso

1. Al abrir la app, YouTube Music carga en el área principal. Inicia sesión con tu cuenta de Google ahí mismo.
2. Busca música o abre tus playlists desde la propia interfaz de YouTube Music.
3. Reproduce una canción: la barra del reproductor oficial queda en la parte inferior del área integrada.
4. Pulsa **Letra** (o `L`) para abrir el panel lateral con la letra sincronizada.
5. Pulsa **Remoto** (o `R`) para mostrar el código QR y escanearlo con el celular.
6. Pulsa **Pantalla grande** (o `F`) para abrir la vista de reproducción a pantalla completa con la letra, la carátula y los controles. Se sale con el botón **Salir** o con `Esc`.

La ventana no usa el marco nativo de Windows: la barra superior es parte de la app. Puedes arrastrar la ventana desde esa zona, hacer doble clic para maximizar y usar los botones de minimizar, maximizar/restaurar y cerrar de la esquina superior derecha. La barra superior incluye el título, los accesos a **Letra**, **Remoto** y **Pantalla grande**, y los controles de ventana.

### Notas importantes

- Todo ocurre en una sola ventana: YouTube Music vive incrustado en el área principal y la letra/control remoto se abren como un panel lateral.
- La primera vez, Windows Defender Firewall puede pedir permiso para el control remoto. Hay que permitirlo en **redes privadas** y conectar el celular a la misma red Wi‑Fi.
- El puerto del control remoto se elige automáticamente entre `47821` y `47860`.
- Cerrar la ventana principal cierra la aplicación.

### Atajos de teclado

Los atajos funcionan cuando el foco está en la barra o en el panel de la app (dentro de YouTube Music se usan los atajos del propio sitio).

| Tecla | Acción |
| --- | --- |
| `Espacio` | Reproducir / pausar |
| `←` / `→` | Retroceder / avanzar 10 segundos |
| `N` / `P` | Siguiente / anterior canción |
| `L` | Abrir / cerrar el panel de letra |
| `R` | Abrir / cerrar el control remoto |
| `F` | Entrar / salir de pantalla grande |
| `Esc` | Salir de pantalla grande |

## Letras no encontradas

- La app busca primero letras sincronizadas en **LRCLIB**.
- Si no las encuentra, intenta obtener la letra sin sincronizar desde la API pública de **Lyrics.ovh**.
- Si tampoco hay resultados, la vista de letra muestra un botón **Buscar en Musixmatch** que abre la búsqueda en el navegador.
- Musixmatch **no ofrece una API pública gratuita** para integrar sus letras sincronizadas: su API es privada/comercial y requiere un convenio. Por eso no se raspa su web ni se usa su contenido sin licencia.

## Cómo funciona

```
┌───────────────────────────────────────────────────────────┐
│ Ventana principal (sin marco nativo)                      │
│ ┌───────────────────────────────┐ ┌─────────────────────┐ │
│ │ WebView de YouTube Music      │ │ Panel Vue           │ │
│ │ · sesión de Google            │ │ · letra sincronizada│ │
│ │ · búsqueda y playlists        │ │ · QR del remoto     │ │
│ │ · reproductor oficial         │ │                     │ │
│ └───────────────┬───────────────┘ └─────────────────────┘ │
└─────────────────┼─────────────────────────────────────────┘
                  │ estado del reproductor
                  ▼
        ┌────────────────────────────────────┐
        │ Backend Rust (Tauri)               │
        │ · estado y comandos del reproductor│
        │ · servidor HTTP para el celular    │
        │ · letras desde LRCLIB (reqwest)    │
        └────────────────────────────────────┘
```

- `src-tauri/src/bridge.js`: se inyecta en `music.youtube.com`. Lee `navigator.mediaSession` y el elemento `<video>`, expone `window.__ytm` con los comandos y envía el estado a la app.
- `src-tauri/src/player.rs`: estado del reproductor y control del webview incrustado de YouTube Music (creación, tamaño, navegación y comandos).
- `src-tauri/src/remote.rs` + `src-tauri/src/remote.html`: servidor y página del control remoto.
- `src-tauri/src/lyrics.rs`: búsqueda de letras sincronizadas en [LRCLIB](https://lrclib.net/) y parseo de LRC.
- `src/App.vue` y `src/components/LyricsView.vue`: barra de título, panel lateral y letra sincronizada.

## Compilar el instalador

```bash
npm run tauri build
```

El instalador queda en `src-tauri/target/release/bundle/`.

## Limitaciones conocidas

- Las letras provienen de LRCLIB (base de datos comunitaria), no del catálogo oficial de YouTube Music. Si una canción no tiene letra sincronizada, se muestra la letra sin tiempos si existe, o un aviso.
- La sincronización es por línea, no palabra por palabra.
- El control remoto actúa sobre el reproductor de YouTube Music incrustado; la app debe estar ejecutándose.
- El acceso al control remoto usa HTTP dentro de la red local y no tiene autenticación: úsalo solo en redes de confianza.
- El inicio de sesión depende de que Google acepte el navegador integrado (WebView2). Si Google pidiera verificación adicional, complétala en la ventana integrada.
