<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import QRCode from "qrcode";
import LyricsView from "./components/LyricsView.vue";
import type { Lyrics, PlayerState, RemoteInfo } from "./types";

function emptyPlayer(): PlayerState {
  return {
    title: "",
    artist: "",
    album: "",
    artwork: "",
    video_id: "",
    duration: 0,
    position: 0,
    playing: false,
    volume: 1,
    updated_at: 0,
  };
}

const player = ref<PlayerState>(emptyPlayer());
const nowTick = ref(performance.now());
const nowWall = ref(Date.now());
const anchorPosition = ref(0);
const anchorAt = ref(performance.now());
const anchorPlaying = ref(false);
const artFailed = ref(false);
const commandError = ref("");
const isMaximized = ref(false);
const bigScreen = ref(false);
const seekDrag = ref<number | null>(null);

const siteSlot = ref<HTMLElement | null>(null);
const siteInited = ref(false);
const siteError = ref("");
const panelOpen = ref(false);
const panelTab = ref<"lyrics" | "remote">("lyrics");

const lyrics = ref<Lyrics | null>(null);
const lyricsLoading = ref(false);
const lyricsKey = ref("");
const lyricsSongKey = ref("");
const lyricsCache = new Map<string, Lyrics>();
let lyricsRequest = 0;
let lyricsTimer: number | undefined;
let lyricsRetryTimer: number | undefined;
let lyricsAttempt = 0;
const MAX_LYRICS_ATTEMPTS = 3;
let errorTimer: number | undefined;

const remote = ref<RemoteInfo | null>(null);
const remoteError = ref("");
const remoteLoading = ref(false);
const qrData = ref("");
const copied = ref(false);

let seekGuardUntil = 0;
let unlisten: UnlistenFn | null = null;
let unlistenResize: UnlistenFn | null = null;
let unlistenScale: UnlistenFn | null = null;
let clockTimer: number | undefined;
let boundsFrame: number | undefined;
let siteObserver: ResizeObserver | null = null;
let bigScreenPoll: number | undefined;

const hasTrack = computed(() => player.value.title.trim().length > 0);

const connected = computed(
  () =>
    player.value.updated_at > 0 &&
    nowWall.value - Number(player.value.updated_at) < 12_000,
);

const position = computed(() => {
  if (seekDrag.value !== null) {
    const duration = player.value.duration;
    return Math.max(
      0,
      duration > 0
        ? Math.min(seekDrag.value, duration)
        : seekDrag.value,
    );
  }

  let value = anchorPosition.value;
  if (anchorPlaying.value) {
    value += (nowTick.value - anchorAt.value) / 1000;
  }
  if (player.value.duration > 0) {
    value = Math.min(value, player.value.duration);
  }
  return Math.max(0, value);
});

const seekValue = computed(() => seekDrag.value ?? position.value);

const progressPercent = computed(() => {
  if (player.value.duration <= 0) return 0;
  return clamp((position.value / player.value.duration) * 100, 0, 100);
});

const seekStyle = computed<Record<string, string>>(() => ({
  "--progress": `${progressPercent.value}%`,
}));

const hasArtwork = computed(() => !!player.value.artwork && !artFailed.value);

const backdropStyle = computed<Record<string, string>>(
  (): Record<string, string> => {
    if (!player.value.artwork) return {};
    return { backgroundImage: `url("${player.value.artwork}")` };
  },
);

const trackKey = computed(() => {
  if (!player.value.title) return "";
  return [
    player.value.video_id,
    player.value.title,
    player.value.artist,
    player.value.album,
    Math.round(player.value.duration),
  ].join("|");
});

function songKeyOf(state: PlayerState): string {
  if (state.video_id) return `v:${state.video_id}`;
  return `m:${state.title}|${state.artist}|${state.album}`;
}

function currentSongKey(): string {
  return songKeyOf(player.value);
}

const activeIndex = computed(() => {
  const data = lyrics.value;
  if (!data || !data.synced || data.lines.length === 0) return -1;

  const target = position.value;
  let low = 0;
  let high = data.lines.length - 1;
  let result = -1;

  while (low <= high) {
    const middle = (low + high) >> 1;
    if (data.lines[middle].time <= target) {
      result = middle;
      low = middle + 1;
    } else {
      high = middle - 1;
    }
  }

  return result;
});

const lineProgress = computed(() => {
  const data = lyrics.value;
  const index = activeIndex.value;
  if (!data || index < 0) return 0;

  const start = data.lines[index].time;
  const next = data.lines[index + 1];
  const end =
    next?.time ??
    (player.value.duration > start ? player.value.duration : start + 4);

  if (end <= start) return 1;
  return clamp((position.value - start) / (end - start), 0, 1);
});

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(value, maximum));
}

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) seconds = 0;
  const total = Math.floor(seconds);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = total % 60;

  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${String(secs).padStart(2, "0")}`;
  }
  return `${minutes}:${String(secs).padStart(2, "0")}`;
}

function applyPlayer(next: PlayerState) {
  const previous = player.value;
  const previousSong = songKeyOf(previous);
  const nextSong = songKeyOf(next);
  const songChanged =
    !!previousSong && !!nextSong && previousSong !== nextSong;

  player.value = next;

  if (songChanged) {
    // Al pasar a otra canción hay que reiniciar el reloj local. En el cambio
    // automático, el estado nuevo puede llegar con la duración/posición de la
    // canción anterior; mostramos 0 hasta que llegue la posición definitiva.
    seekGuardUntil = 0;
    anchorPosition.value = next.position <= 3 ? next.position : 0;
    anchorAt.value = performance.now();
    anchorPlaying.value = next.playing;
    return;
  }

  if (performance.now() < seekGuardUntil) return;

  anchorPosition.value = next.position;
  anchorAt.value = performance.now();
  anchorPlaying.value = next.playing;
}

function flashError(message: string) {
  commandError.value = message;
  window.clearTimeout(errorTimer);
  errorTimer = window.setTimeout(() => {
    commandError.value = "";
  }, 5500);
}

async function send(action: string, value?: number) {
  try {
    await invoke("ytm_command", { action, value: value ?? null });
  } catch (error) {
    flashError(String(error));
  }
}

function togglePlay() {
  const at = position.value;
  const next = !player.value.playing;

  player.value = { ...player.value, playing: next };
  anchorPosition.value = at;
  anchorAt.value = performance.now();
  anchorPlaying.value = next;

  void send(next ? "play" : "pause");
}

function seekTo(seconds: number) {
  const duration = player.value.duration;
  const value = Math.max(0, duration > 0 ? Math.min(seconds, duration) : seconds);

  seekDrag.value = null;
  seekGuardUntil = performance.now() + 900;
  anchorPosition.value = value;
  anchorAt.value = performance.now();
  anchorPlaying.value = player.value.playing;
  player.value = { ...player.value, position: value };

  void send("seek", value);
}

function onSeekInput(event: Event) {
  if (!event.isTrusted) return;
  seekDrag.value = Number((event.target as HTMLInputElement).value);
}

function onSeekChange(event: Event) {
  if (!event.isTrusted) return;
  seekTo(Number((event.target as HTMLInputElement).value));
}

async function enterBigScreen() {
  if (bigScreen.value) return;

  // Activa la vista primero para que cualquier actualización de límites del
  // webview incrustado no vuelva a mostrarlo. El panel lateral no se toca:
  // queda detrás de la vista grande y vuelve a aparecer al salir.
  bigScreen.value = true;

  try {
    await invoke("set_ytm_visible", { visible: false });
  } catch {
    /* el reproductor integrado puede no existir todavía */
  }

  startBigScreenPoll();

  try {
    await getCurrentWindow().setFullscreen(true);
  } catch {
    /* si el sistema no permite fullscreen, usamos la ventana actual */
  }
}

async function exitBigScreen() {
  if (!bigScreen.value) return;

  stopBigScreenPoll();
  bigScreen.value = false;

  try {
    await getCurrentWindow().setFullscreen(false);
  } catch {
    /* ignorar */
  }

  try {
    await invoke("set_ytm_visible", { visible: true });
  } catch {
    /* ignorar */
  }

  void nextTick(scheduleBounds);
}

function startBigScreenPoll() {
  stopBigScreenPoll();
  bigScreenPoll = window.setInterval(() => {
    void invoke("ytm_nudge").catch(() => {
      /* el reproductor puede no estar listo */
    });
  }, 400);
}

function stopBigScreenPoll() {
  if (bigScreenPoll !== undefined) {
    window.clearInterval(bigScreenPoll);
    bigScreenPoll = undefined;
  }
}

function toggleBigScreen() {
  if (bigScreen.value) {
    void exitBigScreen();
  } else {
    void enterBigScreen();
  }
}

function sendBounds() {
  if (!siteInited.value || bigScreen.value) return;
  const slot = siteSlot.value;
  if (!slot) return;

  const rect = slot.getBoundingClientRect();
  void invoke("set_ytm_bounds", {
    x: rect.left,
    y: rect.top,
    width: rect.width,
    height: rect.height,
  }).catch(() => {
    /* la ventana puede estar cerrándose */
  });
}

function scheduleBounds() {
  if (boundsFrame !== undefined) cancelAnimationFrame(boundsFrame);
  boundsFrame = requestAnimationFrame(() => {
    boundsFrame = undefined;
    sendBounds();
  });
}

async function initSite() {
  siteError.value = "";

  const rect = siteSlot.value?.getBoundingClientRect();
  const x = rect?.left ?? 0;
  const y = rect?.top ?? 48;
  const width = rect?.width ?? 1100;
  const height = rect?.height ?? 760;

  try {
    await invoke("init_ytm", { x, y, width, height });
    siteInited.value = true;

    if (bigScreen.value) {
      await invoke("set_ytm_visible", { visible: false });
    }

    scheduleBounds();
  } catch (error) {
    siteInited.value = false;
    siteError.value = String(error);
  }
}

function openPanel(tab: "lyrics" | "remote") {
  if (bigScreen.value) return;

  panelTab.value = tab;
  panelOpen.value = true;

  if (tab === "remote" && !qrData.value && !remoteLoading.value) {
    void loadRemote();
  }

  void nextTick(scheduleBounds);
}

function switchTab(tab: "lyrics" | "remote") {
  panelTab.value = tab;
  if (tab === "remote" && !qrData.value && !remoteLoading.value) {
    void loadRemote();
  }
}

function closePanel() {
  panelOpen.value = false;
  void nextTick(scheduleBounds);
}

async function loadRemote() {
  remoteError.value = "";
  remoteLoading.value = true;

  try {
    const info = await invoke<RemoteInfo>("remote_info");
    remote.value = info;
    qrData.value = await QRCode.toDataURL(info.url, {
      margin: 1,
      width: 400,
      color: { dark: "#0b0b0c", light: "#ffffff" },
    });
  } catch (error) {
    remoteError.value = String(error);
  } finally {
    remoteLoading.value = false;
  }
}

async function copyUrl() {
  if (!remote.value) return;
  try {
    await navigator.clipboard.writeText(remote.value.url);
    copied.value = true;
    window.setTimeout(() => {
      copied.value = false;
    }, 1800);
  } catch {
    copied.value = false;
  }
}

async function minimizeWindow() {
  try {
    await getCurrentWindow().minimize();
  } catch (error) {
    flashError(String(error));
  }
}

async function toggleMaximizeWindow() {
  try {
    const appWindow = getCurrentWindow();
    await appWindow.toggleMaximize();
    isMaximized.value = await appWindow.isMaximized();
  } catch (error) {
    flashError(String(error));
  }
}

async function closeWindow() {
  try {
    await getCurrentWindow().close();
  } catch (error) {
    flashError(String(error));
  }
}

function onKeydown(event: KeyboardEvent) {
  const target = event.target as HTMLElement | null;
  if (
    target &&
    (target.tagName === "INPUT" ||
      target.tagName === "TEXTAREA" ||
      target.isContentEditable)
  ) {
    return;
  }

  if (event.metaKey || event.ctrlKey || event.altKey) return;

  if (bigScreen.value) {
    switch (event.key) {
      case "Escape":
        event.preventDefault();
        void exitBigScreen();
        return;
      case " ":
        event.preventDefault();
        togglePlay();
        return;
      case "ArrowRight":
        event.preventDefault();
        seekTo(position.value + 10);
        return;
      case "ArrowLeft":
        event.preventDefault();
        seekTo(position.value - 10);
        return;
      case "n":
      case "N":
        void send("next");
        return;
      case "p":
      case "P":
        void send("prev");
        return;
      case "f":
      case "F":
        void exitBigScreen();
        return;
    }
    return;
  }

  switch (event.key) {
    case " ":
      event.preventDefault();
      togglePlay();
      break;
    case "ArrowRight":
      event.preventDefault();
      seekTo(position.value + 10);
      break;
    case "ArrowLeft":
      event.preventDefault();
      seekTo(position.value - 10);
      break;
    case "n":
    case "N":
      void send("next");
      break;
    case "p":
    case "P":
      void send("prev");
      break;
    case "l":
    case "L":
      openPanel("lyrics");
      break;
    case "r":
    case "R":
      openPanel("remote");
      break;
    case "f":
    case "F":
      void enterBigScreen();
      break;
  }
}

function clearLyricsRetry() {
  window.clearTimeout(lyricsRetryTimer);
  lyricsRetryTimer = undefined;
}

function scheduleLyricsRetry(song: string): boolean {
  clearLyricsRetry();
  lyricsAttempt += 1;

  if (lyricsAttempt > MAX_LYRICS_ATTEMPTS) return false;

  lyricsLoading.value = true;
  lyricsRetryTimer = window.setTimeout(
    () => {
      if (currentSongKey() !== song) return;
      void loadLyrics(true, true);
    },
    1200 * lyricsAttempt,
  );
  return true;
}

async function loadLyrics(force = false, isRetry = false) {
  const key = trackKey.value;
  const song = currentSongKey();
  if (!key || !song) return;
  if (!force && key === lyricsKey.value && lyrics.value) return;

  if (!isRetry) {
    clearLyricsRetry();
    lyricsAttempt = 0;
  }

  lyricsKey.value = key;
  lyricsSongKey.value = song;
  lyricsRequest += 1;
  const request = lyricsRequest;
  const duration = player.value.duration;
  lyricsLoading.value = true;

  let retrying = false;

  try {
    const result = await invoke<Lyrics>("get_lyrics", {
      title: player.value.title,
      artist: player.value.artist,
      album: player.value.album,
      duration,
    });

    if (request !== lyricsRequest) return;
    if (currentSongKey() !== song) return;

    if (result.lines.length > 0 || result.instrumental) {
      clearLyricsRetry();
      lyricsAttempt = 0;
      lyrics.value = result;
      lyricsCache.set(song, result);
      return;
    }

    // Si no se encontró letra, reintenta antes de mostrar el mensaje.
    if (scheduleLyricsRetry(song)) {
      retrying = true;
      return;
    }

    const cached = lyricsCache.get(song);
    lyrics.value = cached ?? result;
  } catch (error) {
    if (request === lyricsRequest && currentSongKey() === song) {
      if (scheduleLyricsRetry(song)) {
        retrying = true;
        return;
      }

      const cached = lyricsCache.get(song);
      lyrics.value = cached ?? {
        lines: [],
        synced: false,
        instrumental: false,
        source: "",
        message: String(error),
      };
    }
  } finally {
    if (
      request === lyricsRequest &&
      !retrying &&
      currentSongKey() === song
    ) {
      lyricsLoading.value = false;
    }
  }
}

watch(trackKey, (key) => {
  window.clearTimeout(lyricsTimer);
  clearLyricsRetry();

  if (!key) {
    lyricsRequest += 1;
    lyrics.value = null;
    lyricsKey.value = "";
    lyricsSongKey.value = "";
    lyricsLoading.value = false;
    lyricsAttempt = 0;
    return;
  }

  const song = currentSongKey();
  const cached = lyricsCache.get(song);
  const songChanged = song !== lyricsSongKey.value;

  if (songChanged) {
    lyricsRequest += 1;
    lyricsSongKey.value = song;
    lyricsAttempt = 0;
  }

  if (cached) {
    clearLyricsRetry();
    lyricsKey.value = key;
    lyrics.value = cached;
    lyricsLoading.value = false;
    return;
  }

  if (!songChanged && key === lyricsKey.value && lyrics.value) return;

  lyrics.value = null;
  lyricsLoading.value = true;
  lyricsTimer = window.setTimeout(() => {
    void loadLyrics(false);
  }, 400);
});

watch(
  () => player.value.artwork,
  () => {
    artFailed.value = false;
  },
);

onMounted(async () => {
  window.addEventListener("keydown", onKeydown);
  window.addEventListener("resize", scheduleBounds);

  clockTimer = window.setInterval(() => {
    nowTick.value = performance.now();
    nowWall.value = Date.now();
  }, 80);

  if (siteSlot.value) {
    siteObserver = new ResizeObserver(() => scheduleBounds());
    siteObserver.observe(siteSlot.value);
  }

  unlisten = await listen<PlayerState>("player-state", (event) => {
    applyPlayer(event.payload);
  });

  try {
    applyPlayer(await invoke<PlayerState>("get_player_state"));
  } catch {
    /* el backend puede tardar unos milisegundos en montar el estado */
  }

  try {
    const appWindow = getCurrentWindow();
    isMaximized.value = await appWindow.isMaximized();
    unlistenResize = await appWindow.onResized(() => {
      void appWindow
        .isMaximized()
        .then((value) => {
          isMaximized.value = value;
        })
        .catch(() => {});
      scheduleBounds();
    });
    unlistenScale = await appWindow.onScaleChanged(() => scheduleBounds());
  } catch {
    /* fuera de Tauri (vista previa en navegador) */
  }

  await initSite();
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("resize", scheduleBounds);
  siteObserver?.disconnect();
  if (clockTimer) window.clearInterval(clockTimer);
  if (boundsFrame !== undefined) cancelAnimationFrame(boundsFrame);
  window.clearTimeout(lyricsTimer);
  clearLyricsRetry();
  window.clearTimeout(errorTimer);
  stopBigScreenPoll();
  unlisten?.();
  unlistenResize?.();
  unlistenScale?.();
});
</script>

<template>
  <div class="app-shell">
    <div class="backdrop" :style="backdropStyle"></div>

    <header class="topbar" data-tauri-drag-region="deep">
      <div class="topbar-actions">
        <span v-if="!connected" class="status offline">
          <span class="status-dot"></span>
          Desconectado
        </span>

        <span v-if="commandError" class="title-error" :title="commandError">
          {{ commandError }}
        </span>

        <button
          class="title-action"
          :class="{ active: panelOpen && panelTab === 'lyrics' }"
          type="button"
          title="Letra sincronizada (L)"
          @click="openPanel('lyrics')"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M9 18.5a3 3 0 1 1-2-2.83V5l11-2v11.5a3 3 0 1 1-2-2.83V6.2L9 7.6z"
            />
          </svg>
          <span>Letra</span>
        </button>

        <button
          class="title-action"
          :class="{ active: panelOpen && panelTab === 'remote' }"
          type="button"
          title="Control remoto desde el celular (R)"
          @click="openPanel('remote')"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M7 2h10a2 2 0 0 1 2 2v16a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2zm0 2v16h10V4zm4 13h2v1h-2z"
            />
          </svg>
          <span>Remoto</span>
        </button>

        <button
          class="title-action"
          :class="{ active: bigScreen }"
          type="button"
          title="Pantalla grande: letra y carátula (F)"
          @click="toggleBigScreen"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M4 4h6v2H6v4H4zm10 0h6v6h-2V6h-4zM4 14h2v4h4v2H4zm14 0h2v6h-6v-2h4z"
            />
          </svg>
          <span>Pantalla grande</span>
        </button>
      </div>

      <div class="window-controls">
        <button
          class="window-control"
          type="button"
          title="Minimizar"
          aria-label="Minimizar"
          @click="minimizeWindow"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M5 11h14v2H5z" />
          </svg>
        </button>

        <button
          class="window-control"
          type="button"
          :title="isMaximized ? 'Restaurar' : 'Maximizar'"
          :aria-label="isMaximized ? 'Restaurar' : 'Maximizar'"
          @click="toggleMaximizeWindow"
        >
          <svg v-if="isMaximized" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M9 4h11v11h-2V6H9zM4 9h11v11H4zm2 2v7h7v-7z" />
          </svg>
          <svg v-else viewBox="0 0 24 24" aria-hidden="true">
            <path d="M5 5h14v14H5zm2 2v10h10V7z" />
          </svg>
        </button>

        <button
          class="window-control close"
          type="button"
          title="Cerrar"
          aria-label="Cerrar"
          @click="closeWindow"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M6.4 5.2 12 10.8l5.6-5.6 1.2 1.2L13.2 12l5.6 5.6-1.2 1.2L12 13.2l-5.6 5.6-1.2-1.2L10.8 12 5.2 6.4z"
            />
          </svg>
        </button>
      </div>
    </header>

    <div class="workspace">
      <div ref="siteSlot" class="browser-slot">
        <div v-if="siteError" class="slot-message">
          <strong>No se pudo cargar YouTube Music</strong>
          <span>{{ siteError }}</span>
          <button class="btn primary" type="button" @click="initSite">
            Reintentar
          </button>
        </div>
        <div v-else-if="!siteInited" class="slot-message">
          <span class="spinner"></span>
          <strong>Cargando YouTube Music…</strong>
          <span>Tu biblioteca, búsquedas y playlists aparecerán aquí.</span>
        </div>
      </div>

      <aside v-if="panelOpen" class="side-panel">
        <div class="panel-head">
          <div class="panel-tabs">
            <button
              class="panel-tab"
              :class="{ active: panelTab === 'lyrics' }"
              type="button"
              @click="switchTab('lyrics')"
            >
              Letra
            </button>
            <button
              class="panel-tab"
              :class="{ active: panelTab === 'remote' }"
              type="button"
              @click="switchTab('remote')"
            >
              Remoto
            </button>
          </div>

          <span v-if="panelTab === 'lyrics' && lyrics?.source" class="badge">
            {{ lyrics.source }}
          </span>

          <button
            v-if="panelTab === 'lyrics'"
            class="panel-icon"
            type="button"
            title="Volver a buscar la letra"
            aria-label="Volver a buscar la letra"
            :disabled="!hasTrack || lyricsLoading"
            @click="loadLyrics(true)"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M12 5V2L7 6l5 4V7a5 5 0 1 1-5 5H5a7 7 0 1 0 7-7z" />
            </svg>
          </button>

          <button
            class="panel-icon"
            type="button"
            title="Pantalla grande"
            aria-label="Pantalla grande"
            @click="toggleBigScreen"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path
                d="M4 4h6v2H6v4H4zm10 0h6v6h-2V6h-4zM4 14h2v4h4v2H4zm14 0h2v6h-6v-2h4z"
              />
            </svg>
          </button>

          <button
            class="panel-icon"
            type="button"
            title="Cerrar panel"
            aria-label="Cerrar panel"
            @click="closePanel"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path
                d="M6.4 5.2 12 10.8l5.6-5.6 1.2 1.2L13.2 12l5.6 5.6-1.2 1.2L12 13.2l-5.6 5.6-1.2-1.2L10.8 12 5.2 6.4z"
              />
            </svg>
          </button>
        </div>

        <div class="panel-now">
          <div class="panel-cover" :class="{ empty: !hasArtwork }">
            <img
              v-if="hasArtwork"
              :src="player.artwork"
              alt="Portada"
              draggable="false"
              @error="artFailed = true"
            />
            <svg v-else viewBox="0 0 24 24" aria-hidden="true">
              <path
                d="M9 18.5a3 3 0 1 1-2-2.83V5l11-2v11.5a3 3 0 1 1-2-2.83V6.2L9 7.6z"
              />
            </svg>
          </div>
          <div class="panel-meta">
            <strong :title="player.title">
              {{ player.title || "Sin reproducción" }}
            </strong>
            <span :title="player.artist">
              {{ player.artist || "Elige una canción en YouTube Music" }}
            </span>
          </div>
        </div>

        <div class="panel-body" :class="{ remote: panelTab === 'remote' }">
          <LyricsView
            v-if="panelTab === 'lyrics'"
            :lines="lyrics?.lines ?? []"
            :synced="lyrics?.synced ?? false"
            :instrumental="lyrics?.instrumental ?? false"
            :active-index="activeIndex"
            :progress="lineProgress"
            :loading="lyricsLoading"
            :has-track="hasTrack"
            :message="lyrics?.message ?? ''"
            @seek="seekTo"
          />

          <div v-else class="remote-wrap">
            <p v-if="remoteError" class="error">{{ remoteError }}</p>

            <div v-else-if="remoteLoading || !remote" class="remote-loading">
              <span class="spinner"></span>
              <span>Preparando el control remoto…</span>
            </div>

            <template v-else>
              <p class="remote-copy">
                Escanea el código con la cámara de tu celular. Ambos equipos
                deben estar en la misma red Wi‑Fi.
              </p>

              <div class="qr">
                <img
                  v-if="qrData"
                  :src="qrData"
                  alt="Código QR del control remoto"
                />
              </div>

              <div class="url-row">
                <code>{{ remote.url }}</code>
                <button type="button" @click="copyUrl">
                  {{ copied ? "Copiado" : "Copiar" }}
                </button>
              </div>

              <ul class="hints">
                <li>La aplicación debe estar abierta en el PC.</li>
                <li>
                  Si Windows pregunta por el firewall, permite el acceso en
                  redes privadas.
                </li>
                <li>
                  Desde el celular puedes pausar, cambiar de canción, mover la
                  barra y ajustar el volumen.
                </li>
              </ul>
            </template>
          </div>
        </div>
      </aside>
    </div>

    <div v-if="bigScreen" class="big-screen">
      <div class="big-bg" :style="backdropStyle"></div>

      <div class="big-topbar">
        <button
          class="big-exit"
          type="button"
          title="Salir (Esc)"
          aria-label="Salir de pantalla grande"
          @click="exitBigScreen"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M9 4v5H4V7h3V4zm6 0h2v3h3v2h-5zM4 15h5v5H7v-3H4zm11 0h5v2h-3v3h-2z"
            />
          </svg>
        </button>
      </div>

      <div class="big-stage">
        <section class="big-lyrics">
          <LyricsView
            :lines="lyrics?.lines ?? []"
            :synced="lyrics?.synced ?? false"
            :instrumental="lyrics?.instrumental ?? false"
            :active-index="activeIndex"
            :progress="lineProgress"
            :loading="lyricsLoading"
            :has-track="hasTrack"
            :message="lyrics?.message ?? ''"
            :windowed="true"
            @seek="seekTo"
          />
        </section>

        <aside class="big-info">
          <div class="big-cover" :class="{ empty: !hasArtwork }">
            <img
              v-if="hasArtwork"
              :src="player.artwork"
              alt="Portada del álbum"
              draggable="false"
              @error="artFailed = true"
            />
            <svg v-else viewBox="0 0 24 24" aria-hidden="true">
              <path
                d="M9 18.5a3 3 0 1 1-2-2.83V5l11-2v11.5a3 3 0 1 1-2-2.83V6.2L9 7.6z"
              />
            </svg>
          </div>

          <div class="big-meta">
            <h1 :title="player.title">{{ player.title || "Sin reproducción" }}</h1>
            <p class="big-artist">
              {{ player.artist || "Elige una canción en YouTube Music" }}
            </p>
            <p v-if="player.album" class="big-album">{{ player.album }}</p>
          </div>

          <div class="big-controls">
            <button
              class="big-ctrl"
              type="button"
              title="Canción anterior (P)"
              @click="send('prev')"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="M6 5h2v14H6zm3.5 7L19 5v14z" />
              </svg>
            </button>

            <button
              class="big-play"
              type="button"
              :title="player.playing ? 'Pausar (espacio)' : 'Reproducir (espacio)'"
              @click="togglePlay"
            >
              <svg v-if="player.playing" viewBox="0 0 24 24" aria-hidden="true">
                <path d="M7 5h4v14H7zm6 0h4v14h-4z" />
              </svg>
              <svg v-else viewBox="0 0 24 24" aria-hidden="true">
                <path d="M8 5v14l11-7z" />
              </svg>
            </button>

            <button
              class="big-ctrl"
              type="button"
              title="Canción siguiente (N)"
              @click="send('next')"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="M16 5h2v14h-2zM5 5l10.5 7L5 19z" />
              </svg>
            </button>
          </div>

          <div class="big-progress">
            <span>{{ formatTime(seekValue) }}</span>
            <input
              class="big-seek"
              type="range"
              min="0"
              :max="player.duration || 1"
              step="0.1"
              :value="seekValue"
              :disabled="!hasTrack || player.duration <= 0"
              :style="seekStyle"
              aria-label="Posición de la canción"
              @input="onSeekInput"
              @change="onSeekChange"
            />
            <span>{{ formatTime(player.duration) }}</span>
          </div>

        </aside>
      </div>

      <p v-if="commandError" class="big-error">{{ commandError }}</p>
    </div>
  </div>
</template>

<style>
:root {
  color-scheme: dark;
  --bg: #08080a;
  --text: #f5f5f7;
  --muted: #9b9ba6;
  --line: rgba(255, 255, 255, 0.09);
  --red: #ff3b5c;
  --red-strong: #ff0033;
}

* {
  box-sizing: border-box;
}

html,
body,
#app {
  height: 100%;
}

body {
  margin: 0;
  overflow: hidden;
  background: var(--bg);
  color: var(--text);
  font-family:
    "Segoe UI Variable Display", "Segoe UI", Inter, system-ui, -apple-system,
    Roboto, Helvetica, Arial, sans-serif;
  -webkit-font-smoothing: antialiased;
  text-rendering: optimizeLegibility;
}

button,
input {
  font: inherit;
}

button {
  -webkit-tap-highlight-color: transparent;
}

::selection {
  background: rgba(255, 59, 92, 0.4);
}
</style>

<style scoped>
.app-shell {
  position: relative;
  height: 100%;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.backdrop {
  position: absolute;
  inset: -12%;
  z-index: 0;
  background-size: cover;
  background-position: center;
  filter: blur(100px) saturate(1.35);
  opacity: 0.3;
  transform: scale(1.14);
  transition:
    background-image 0.7s ease,
    opacity 0.7s ease;
  pointer-events: none;
}

.backdrop::after {
  content: "";
  position: absolute;
  inset: 0;
  background:
    radial-gradient(circle at 30% 10%, rgba(8, 8, 10, 0.3), transparent 58%),
    linear-gradient(180deg, rgba(8, 8, 10, 0.45), rgba(8, 8, 10, 0.95) 88%);
}

.topbar {
  position: relative;
  z-index: 3;
  flex: none;
  height: 48px;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 0 0 0 18px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.055);
  background: linear-gradient(
    180deg,
    rgba(8, 8, 10, 0.72),
    rgba(8, 8, 10, 0.4)
  );
  backdrop-filter: blur(18px);
  user-select: none;
}

.topbar-actions {
  display: flex;
  align-items: center;
  gap: 5px;
  min-width: 0;
  margin-left: auto;
}

.status {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 0 8px 0 2px;
  font-size: 11.5px;
  color: #ff9aa9;
  white-space: nowrap;
}

.status-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: #ff4d5e;
  box-shadow: 0 0 10px rgba(255, 77, 94, 0.6);
}

.title-error {
  max-width: 210px;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  padding: 3px 8px;
  border: 1px solid rgba(255, 59, 92, 0.25);
  border-radius: 999px;
  background: rgba(255, 59, 92, 0.15);
  color: #ffb3c0;
  font-size: 10.5px;
  font-weight: 600;
}

.title-action {
  height: 30px;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 0 10px;
  border: 1px solid transparent;
  border-radius: 8px;
  background: transparent;
  color: rgba(255, 255, 255, 0.58);
  font-size: 11.5px;
  font-weight: 650;
  white-space: nowrap;
  cursor: pointer;
  transition:
    background 0.15s ease,
    border-color 0.15s ease,
    color 0.15s ease;
}

.title-action:hover {
  background: rgba(255, 255, 255, 0.06);
  color: #fff;
}

.title-action.active {
  border-color: rgba(255, 59, 92, 0.22);
  background: rgba(255, 59, 92, 0.13);
  color: #ff9aac;
}

.title-action svg {
  width: 15px;
  height: 15px;
  fill: currentColor;
  flex: none;
}

.window-controls {
  display: flex;
  align-self: stretch;
  flex: none;
  margin-left: 4px;
}

.window-control {
  width: 46px;
  height: 100%;
  display: grid;
  place-items: center;
  border: 0;
  border-radius: 0;
  background: transparent;
  color: rgba(255, 255, 255, 0.7);
  cursor: pointer;
  transition:
    background 0.15s ease,
    color 0.15s ease;
}

.window-control svg {
  width: 12px;
  height: 12px;
  fill: currentColor;
}

.window-control:hover {
  background: rgba(255, 255, 255, 0.09);
  color: #fff;
}

.window-control:active {
  background: rgba(255, 255, 255, 0.15);
}

.window-control.close:hover {
  background: #e81123;
  color: #fff;
}

.window-control.close:active {
  background: #c50f1f;
  color: #fff;
}

.workspace {
  position: relative;
  z-index: 1;
  flex: 1;
  min-height: 0;
  display: flex;
  overflow: hidden;
}

.browser-slot {
  position: relative;
  flex: 1;
  min-width: 0;
  min-height: 0;
  background: #0c0c0f;
}

.slot-message {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 24px;
  text-align: center;
  background:
    radial-gradient(circle at 50% 30%, rgba(255, 59, 92, 0.08), transparent 55%),
    linear-gradient(160deg, #101014, #08080a);
}

.slot-message strong {
  font-size: 14px;
  font-weight: 700;
}

.slot-message span:not(.spinner) {
  max-width: 360px;
  font-size: 12px;
  line-height: 1.55;
  color: var(--muted);
}

.slot-message .btn {
  margin-top: 6px;
}

.spinner {
  width: 22px;
  height: 22px;
  border-radius: 50%;
  border: 2px solid rgba(255, 255, 255, 0.15);
  border-top-color: #fff;
  animation: spin 0.85s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.side-panel {
  --lyric-size: clamp(16px, 1.45vw, 23px);
  --lyric-gap: 15px;
  width: 420px;
  flex: none;
  display: flex;
  flex-direction: column;
  min-height: 0;
  border-left: 1px solid rgba(255, 255, 255, 0.085);
  background: linear-gradient(
    180deg,
    rgba(17, 17, 21, 0.96),
    rgba(8, 8, 10, 0.985)
  );
  box-shadow: -22px 0 60px rgba(0, 0, 0, 0.34);
}

.panel-head {
  flex: none;
  height: 48px;
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 0 11px 0 14px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
}

.panel-tabs {
  display: flex;
  gap: 2px;
  padding: 3px;
  border: 1px solid rgba(255, 255, 255, 0.07);
  border-radius: 10px;
  background: rgba(255, 255, 255, 0.025);
}

.panel-tab {
  height: 27px;
  padding: 0 11px;
  border: 0;
  border-radius: 7px;
  background: transparent;
  color: var(--muted);
  font-size: 11.5px;
  font-weight: 650;
  white-space: nowrap;
  cursor: pointer;
  transition:
    background 0.15s ease,
    color 0.15s ease;
}

.panel-tab:hover {
  color: #fff;
}

.panel-tab.active {
  background: rgba(255, 255, 255, 0.1);
  color: #fff;
}

.badge {
  margin-left: auto;
  padding: 3px 7px;
  border-radius: 999px;
  border: 1px solid rgba(255, 59, 92, 0.2);
  background: rgba(255, 59, 92, 0.12);
  color: #ff95a8;
  font-size: 8.5px;
  font-weight: 700;
  letter-spacing: 0.07em;
  text-transform: uppercase;
  white-space: nowrap;
}

.panel-icon {
  width: 30px;
  height: 30px;
  flex: none;
  display: grid;
  place-items: center;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  transition:
    background 0.15s ease,
    color 0.15s ease;
}

.panel-icon:hover {
  background: rgba(255, 255, 255, 0.07);
  color: #fff;
}

.panel-icon:disabled {
  opacity: 0.32;
  cursor: default;
}

.panel-icon svg {
  width: 15px;
  height: 15px;
  fill: currentColor;
}

.panel-now {
  flex: none;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 11px 15px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.055);
}

.panel-cover {
  width: 46px;
  height: 46px;
  flex: none;
  border-radius: 10px;
  overflow: hidden;
  background: #15151a;
  box-shadow: 0 8px 20px rgba(0, 0, 0, 0.35);
}

.panel-cover img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.panel-cover.empty {
  display: grid;
  place-items: center;
  background:
    radial-gradient(circle at 30% 20%, rgba(255, 59, 92, 0.25), transparent 60%),
    linear-gradient(145deg, #1b1b21, #0c0c10);
  color: rgba(255, 255, 255, 0.22);
}

.panel-cover svg {
  width: 22px;
  height: 22px;
  fill: currentColor;
}

.panel-meta {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.panel-meta strong {
  font-size: 13.5px;
  font-weight: 700;
  letter-spacing: -0.012em;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.panel-meta span {
  font-size: 11.5px;
  color: var(--muted);
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.panel-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  padding: 0 15px 10px;
}

.panel-body.remote {
  display: block;
  overflow-y: auto;
  padding-top: 13px;
  scrollbar-width: thin;
  scrollbar-color: rgba(255, 255, 255, 0.16) transparent;
}

.remote-wrap {
  display: flex;
  flex-direction: column;
  gap: 13px;
}

.remote-copy {
  margin: 0;
  font-size: 12px;
  line-height: 1.55;
  color: var(--muted);
}

.remote-loading {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  padding: 44px 20px;
  color: var(--muted);
  font-size: 12px;
  text-align: center;
}

.qr {
  display: grid;
  place-items: center;
  padding: 3px 0;
}

.qr img {
  width: 218px;
  height: 218px;
  padding: 6px;
  border-radius: 14px;
  background: #fff;
  box-shadow: 0 16px 40px rgba(0, 0, 0, 0.42);
}

.url-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 5px 5px 10px;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: rgba(255, 255, 255, 0.04);
}

.url-row code {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11.5px;
  color: #cfcfd8;
}

.url-row button {
  height: 27px;
  padding: 0 9px;
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 7px;
  background: rgba(255, 255, 255, 0.07);
  color: var(--text);
  font-size: 11px;
  font-weight: 650;
  cursor: pointer;
}

.url-row button:hover {
  background: rgba(255, 255, 255, 0.12);
}

.hints {
  margin: 0;
  padding-left: 16px;
  font-size: 11.5px;
  line-height: 1.65;
  color: var(--muted);
}

.hints li {
  margin-bottom: 3px;
}

.error {
  margin: 0;
  font-size: 12px;
  line-height: 1.5;
  color: #ff9aa9;
}

.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  height: 34px;
  padding: 0 14px;
  border-radius: 9px;
  border: 1px solid var(--line);
  background: rgba(255, 255, 255, 0.06);
  color: var(--text);
  font-size: 12.5px;
  font-weight: 620;
  white-space: nowrap;
  cursor: pointer;
}

.btn:hover {
  background: rgba(255, 255, 255, 0.1);
}

.btn.primary {
  border-color: transparent;
  background: linear-gradient(135deg, var(--red), var(--red-strong));
  box-shadow: 0 12px 28px rgba(255, 0, 51, 0.28);
}

.big-screen {
  position: fixed;
  inset: 0;
  z-index: 60;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background:
    radial-gradient(circle at 28% 24%, rgba(255, 59, 92, 0.16), transparent 55%),
    #050507;
}

.big-bg {
  position: absolute;
  inset: -18%;
  background-size: cover;
  background-position: center;
  filter: blur(40px) saturate(1.5);
  opacity: 0.86;
  transform: scale(1.22);
  pointer-events: none;
}

.big-bg::after {
  content: "";
  position: absolute;
  inset: 0;
  background:
    linear-gradient(
      90deg,
      rgba(5, 5, 7, 0.78) 0%,
      rgba(5, 5, 7, 0.5) 42%,
      rgba(5, 5, 7, 0.2) 72%,
      rgba(5, 5, 7, 0.32) 100%
    ),
    linear-gradient(180deg, rgba(5, 5, 7, 0.12), rgba(5, 5, 7, 0.56) 100%);
}

.big-topbar {
  position: relative;
  z-index: 2;
  flex: none;
  height: 58px;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  padding: 12px clamp(28px, 4vw, 64px) 0;
}

.big-exit {
  width: 38px;
  height: 38px;
  display: grid;
  place-items: center;
  padding: 0;
  border: 1px solid rgba(255, 255, 255, 0.14);
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.08);
  color: rgba(255, 255, 255, 0.82);
  cursor: pointer;
  backdrop-filter: blur(12px);
  transition:
    background 0.15s ease,
    color 0.15s ease,
    transform 0.12s ease;
}

.big-exit:hover {
  background: rgba(255, 255, 255, 0.15);
  color: #fff;
}

.big-exit:active {
  transform: scale(0.94);
}

.big-exit svg {
  width: 16px;
  height: 16px;
  fill: currentColor;
}

.big-stage {
  position: relative;
  z-index: 1;
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(0, 1.55fr) minmax(310px, 0.8fr);
  gap: clamp(36px, 6vw, 96px);
  padding: 0 clamp(32px, 5vw, 80px) clamp(30px, 5vh, 56px);
  align-items: stretch;
}

.big-lyrics {
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
  --lyric-size: clamp(30px, 3.4vw, 56px);
  --lyric-gap: clamp(16px, 1.9vh, 28px);
}

.big-lyrics :deep(.line) {
  text-shadow: 0 2px 22px rgba(0, 0, 0, 0.42);
}

.big-info {
  align-self: center;
  justify-self: center;
  width: 100%;
  max-width: 440px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 16px;
}

.big-cover {
  width: min(37vh, 390px);
  aspect-ratio: 1;
  border-radius: 24px;
  overflow: hidden;
  background: #15151a;
  box-shadow:
    0 34px 80px rgba(0, 0, 0, 0.58),
    0 0 0 1px rgba(255, 255, 255, 0.07) inset;
}

.big-cover img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
  user-select: none;
}

.big-cover.empty {
  display: grid;
  place-items: center;
  background:
    radial-gradient(circle at 30% 18%, rgba(255, 59, 92, 0.3), transparent 58%),
    radial-gradient(circle at 76% 82%, rgba(92, 70, 255, 0.28), transparent 58%),
    linear-gradient(145deg, #1b1b21, #0c0c10);
  color: rgba(255, 255, 255, 0.22);
}

.big-cover.empty svg {
  width: 52px;
  height: 52px;
  fill: currentColor;
}

.big-meta {
  width: 100%;
  text-align: center;
}

.big-meta h1 {
  margin: 0;
  font-size: clamp(25px, 2.6vw, 42px);
  font-weight: 800;
  line-height: 1.12;
  letter-spacing: -0.032em;
  overflow: hidden;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow-wrap: anywhere;
  text-shadow: 0 2px 22px rgba(0, 0, 0, 0.45);
}

.big-artist {
  margin: 8px 0 0;
  font-size: clamp(14px, 1.15vw, 18px);
  color: rgba(255, 255, 255, 0.72);
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  text-shadow: 0 2px 16px rgba(0, 0, 0, 0.45);
}

.big-album {
  margin: 4px 0 0;
  font-size: 11.5px;
  color: var(--muted);
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.big-controls {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 18px;
  margin-top: 2px;
}

.big-ctrl {
  width: 50px;
  height: 50px;
  display: grid;
  place-items: center;
  border: 0;
  border-radius: 50%;
  background: transparent;
  color: rgba(255, 255, 255, 0.8);
  cursor: pointer;
  transition:
    background 0.15s ease,
    color 0.15s ease,
    transform 0.12s ease;
}

.big-ctrl:hover {
  background: rgba(255, 255, 255, 0.1);
  color: #fff;
}

.big-ctrl:active {
  transform: scale(0.94);
}

.big-ctrl svg {
  width: 24px;
  height: 24px;
  fill: currentColor;
}

.big-play {
  width: 66px;
  height: 66px;
  display: grid;
  place-items: center;
  border: 0;
  border-radius: 50%;
  background: #fff;
  color: #0a0a0c;
  cursor: pointer;
  box-shadow: 0 16px 40px rgba(0, 0, 0, 0.45);
  transition: transform 0.14s ease;
}

.big-play:hover {
  transform: scale(1.04);
}

.big-play svg {
  width: 27px;
  height: 27px;
  fill: currentColor;
}

.big-progress {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 11px;
  color: rgba(255, 255, 255, 0.55);
  font-variant-numeric: tabular-nums;
}

.big-progress span {
  width: 38px;
  flex: none;
}

.big-progress span:last-child {
  text-align: right;
}

.big-seek {
  -webkit-appearance: none;
  appearance: none;
  flex: 1;
  min-width: 0;
  height: 18px;
  margin: 0;
  background: transparent;
  cursor: pointer;
}

.big-seek::-webkit-slider-runnable-track {
  height: 5px;
  border-radius: 999px;
  background: linear-gradient(
    90deg,
    #fff var(--progress, 0%),
    rgba(255, 255, 255, 0.18) var(--progress, 0%)
  );
}

.big-seek::-webkit-slider-thumb {
  -webkit-appearance: none;
  appearance: none;
  width: 14px;
  height: 14px;
  margin-top: -4.5px;
  border: none;
  border-radius: 50%;
  background: #fff;
  opacity: 0;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.55);
  transition: opacity 0.15s ease;
}

.big-seek:hover::-webkit-slider-thumb,
.big-seek:focus-visible::-webkit-slider-thumb {
  opacity: 1;
}

.big-seek::-moz-range-track {
  height: 5px;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.18);
}

.big-seek::-moz-range-progress {
  height: 5px;
  border-radius: 999px;
  background: #fff;
}

.big-seek::-moz-range-thumb {
  width: 14px;
  height: 14px;
  border: none;
  border-radius: 50%;
  background: #fff;
}

.big-seek:disabled {
  opacity: 0.4;
  cursor: default;
}

.big-error {
  position: absolute;
  z-index: 10;
  left: 50%;
  bottom: 16px;
  transform: translateX(-50%);
  max-width: min(560px, 70%);
  margin: 0;
  padding: 8px 14px;
  border: 1px solid rgba(255, 59, 92, 0.25);
  border-radius: 10px;
  background: rgba(35, 10, 16, 0.94);
  color: #ffb3c0;
  font-size: 12px;
  text-align: center;
  box-shadow: 0 16px 40px rgba(0, 0, 0, 0.5);
}

@media (max-width: 1100px) {
  .big-stage {
    grid-template-columns: minmax(0, 1.4fr) minmax(280px, 0.8fr);
    gap: 28px;
    padding-left: 32px;
    padding-right: 32px;
  }

  .big-lyrics {
    --lyric-size: clamp(26px, 3.6vw, 44px);
  }

  .big-cover {
    width: min(33vh, 320px);
  }
}

@media (max-height: 700px) {
  .big-topbar {
    height: 50px;
    padding-top: 10px;
  }

  .big-cover {
    width: min(32vh, 260px);
  }

  .big-info {
    gap: 11px;
  }

  .big-controls {
    gap: 12px;
  }

  .big-play {
    width: 58px;
    height: 58px;
  }

  .big-lyrics {
    --lyric-size: clamp(24px, 3vw, 40px);
    --lyric-gap: 14px;
  }
}

@media (max-width: 1180px) {
  .side-panel {
    width: 380px;
    --lyric-size: clamp(15px, 1.6vw, 20px);
  }

  .qr img {
    width: 200px;
    height: 200px;
  }
}

@media (max-height: 760px) {
  .topbar,
  .panel-head {
    height: 44px;
  }

  .panel-now {
    padding: 8px 15px;
  }

  .panel-cover {
    width: 40px;
    height: 40px;
  }
}
</style>
