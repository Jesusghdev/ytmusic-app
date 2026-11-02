# YouTube Music Desktop v0.2.0

Primer release público para **Windows x64**.

## Descargar

- **Instalador (recomendado):** archivo `*-setup.exe` de esta release.
- **Portable:** archivo `*-portable-windows-x64.zip`; descomprime y ejecuta `YouTube Music Desktop.exe`.

## Requisitos

- Windows 10/11 de 64 bits.
- Microsoft Edge WebView2 (incluido en Windows 11; el instalador puede descargarlo si falta).
- Una cuenta de Google para iniciar sesión en YouTube Music.

## Qué incluye

- YouTube Music integrado en la misma ventana: búsqueda, biblioteca, playlists y reproductor oficial.
- Letra sincronizada en un panel lateral y en el modo **Pantalla grande**.
- Controles de reproducción, progreso y carátula.
- Control remoto desde el celular mediante código QR.
- Atajos de teclado y barra de título personalizada.
- Letras desde LRCLIB (sincronizadas) y Lyrics.ovh (sin sincronizar), con reintentos automáticos.

## Notas importantes

- **SmartScreen:** la aplicación no está firmada digitalmente. Windows puede mostrar “Windows protegió tu PC”; pulsa **Más información → Ejecutar de todas formas**.
- **Proyecto no oficial:** no está afiliado a Google ni a YouTube. YouTube Music y su logo son marcas de Google. Úsalo bajo tu responsabilidad.
- **Control remoto:** el servidor HTTP local no tiene autenticación. Úsalo solo en redes de confianza. Si Windows Firewall pregunta, permite el acceso en redes privadas.
- **Sesión:** el inicio de sesión se hace en la web oficial dentro de WebView2. La sesión se guarda localmente en el equipo, no en este repositorio ni en el instalador.
- **Letras:** no todas las canciones tienen letra disponible; la cobertura depende de las fuentes públicas.
- **Sin actualizaciones automáticas:** las próximas versiones se descargan manualmente desde Releases.
- Primer release: puede contener errores. Los reportes e ideas son bienvenidos en Issues.

## Instalación rápida

1. Descarga el instalador (o el ZIP portable).
2. Ejecútalo y permite el aviso de SmartScreen si aparece.
3. Abre la app, inicia sesión con tu cuenta de Google y elige música.
4. Para el control remoto, pulsa **Remoto** y escanea el QR con el celular (misma red Wi‑Fi).
