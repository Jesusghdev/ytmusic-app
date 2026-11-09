<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { LyricLine } from "../types";

const props = defineProps<{
  lines: LyricLine[];
  synced: boolean;
  instrumental: boolean;
  activeIndex: number;
  progress: number;
  loading: boolean;
  hasTrack: boolean;
  message: string;
  windowed?: boolean;
  windowedLines?: number;
  hideScrollbar?: boolean;
  highlightMode?: "progressive" | "full";
}>();

const emit = defineEmits<{
  (event: "seek", time: number): void;
}>();

const viewport = ref<HTMLElement | null>(null);
const offset = ref(0);
const activeStyle = computed(() => ({
  "--p": `${Math.max(0, Math.min(1, props.progress)) * 100}%`,
}) as Record<string, string>);

let resizeObserver: ResizeObserver | null = null;
let manualScrollUntil = 0;
let manualScrollTimer: number | undefined;
let programmaticScrollUntil = 0;

const MANUAL_SCROLL_PAUSE = 5000;

function onScroll() {
  if (!props.windowed || performance.now() < programmaticScrollUntil) return;

  manualScrollUntil = performance.now() + MANUAL_SCROLL_PAUSE;
  window.clearTimeout(manualScrollTimer);
  manualScrollTimer = window.setTimeout(() => {
    manualScrollUntil = 0;
    centerActiveLine();
  }, MANUAL_SCROLL_PAUSE);
}

function centerActiveLine() {
  if (!props.synced || props.lines.length === 0) {
    offset.value = 0;
    return;
  }

  const targetIndex = Math.max(props.activeIndex, 0);
  const container = viewport.value;
  const line = container?.querySelector<HTMLElement>(
    `[data-line="${targetIndex}"]`,
  );

  if (!container || !line) return;
  if (props.windowed && performance.now() < manualScrollUntil) return;

  const target =
    line.offsetTop + line.offsetHeight / 2 - container.clientHeight * 0.42;
  if (props.windowed) {
    const maximum = Math.max(0, container.scrollHeight - container.clientHeight);
    programmaticScrollUntil = performance.now() + 700;
    container.scrollTo({
      top: Math.max(0, Math.min(target, maximum)),
      behavior: "smooth",
    });
    offset.value = 0;
    return;
  }

  offset.value = Math.max(0, target);
}

watch(
  () => props.activeIndex,
  () => {
    void nextTick(centerActiveLine);
  },
);

watch(
  () => props.lines,
  () => {
    offset.value = 0;
    void nextTick(centerActiveLine);
  },
);

watch(
  () => props.synced,
  () => {
    void nextTick(centerActiveLine);
  },
);

function onLine(line: LyricLine) {
  if (!props.synced || line.time < 0) return;
  emit("seek", line.time);
}

function isOutsideWindow(index: number): boolean {
  if (!props.windowed || !props.synced) return false;

  const active = props.activeIndex;
  if (active < 0 || active >= props.lines.length) return index > 2;

  const visibleBefore = props.windowedLines ?? 1;
  const visibleAfter = (props.windowedLines ?? 2) + 1;
  return index < active - visibleBefore || index > active + visibleAfter;
}

onMounted(() => {
  void nextTick(centerActiveLine);
});

watch(viewport, (element) => {
  resizeObserver?.disconnect();
  resizeObserver = null;

  if (!element || typeof ResizeObserver === "undefined") return;
  resizeObserver = new ResizeObserver(() => centerActiveLine());
  resizeObserver.observe(element);
  void nextTick(centerActiveLine);
});

onBeforeUnmount(() => {
  resizeObserver?.disconnect();
  window.clearTimeout(manualScrollTimer);
});
</script>

<template>
  <div
    class="lyrics-shell"
    :class="{ 'full-highlight': highlightMode === 'full' }"
  >
    <div v-if="loading" class="state">
      <p>Buscando letra…</p>
      <span>Sincronizando con la canción</span>
    </div>

    <div v-else-if="!hasTrack" class="state">
      <p>La letra aparecerá aquí</p>
      <span>Reproduce una canción en YouTube Music.</span>
    </div>

    <div v-else-if="instrumental" class="state">
      <p>♪ Instrumental</p>
      <span>Esta canción no tiene letra.</span>
    </div>

    <div v-else-if="!lines.length" class="state">
      <p>Sin letra disponible</p>
      <span>{{ message || "No encontramos la letra para esta canción." }}</span>
    </div>

    <div
      v-else
      ref="viewport"
      class="lyrics-viewport"
      :class="{
        static: !synced,
        windowed: windowed && synced,
        'hide-scrollbar': hideScrollbar,
      }"
      @scroll="onScroll"
    >
      <div
        class="lyrics-track"
        :style="
          synced && !windowed
            ? { transform: `translate3d(0, ${-offset}px, 0)` }
            : undefined
        "
      >
        <p
          v-for="(line, index) in lines"
          :key="`${index}-${line.time}`"
          :data-line="index"
          class="line"
          :class="{
            active: synced && index === activeIndex,
            past: synced && index < activeIndex,
            clickable: synced,
            'out-of-window': isOutsideWindow(index),
          }"
          :style="synced && index === activeIndex ? activeStyle : undefined"
          @click="onLine(line)"
        >
          <span>{{ line.text || "♪" }}</span>
        </p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.lyrics-shell {
  position: relative;
  display: flex;
  flex-direction: column;
  min-height: 0;
  flex: 1;
}

.state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  gap: 6px;
  padding: 24px;
}

.state p {
  margin: 0;
  font-size: 19px;
  font-weight: 650;
  color: rgba(255, 255, 255, 0.72);
}

.state span {
  font-size: 13px;
  color: var(--muted);
  max-width: 320px;
  line-height: 1.5;
}

.lyrics-viewport {
  position: relative;
  flex: 1;
  min-height: 0;
  overflow: hidden;
  padding: 0 8px 0 4px;
  mask-image: linear-gradient(
    to bottom,
    transparent 0%,
    #000 10%,
    #000 84%,
    transparent 100%
  );
  -webkit-mask-image: linear-gradient(
    to bottom,
    transparent 0%,
    #000 10%,
    #000 84%,
    transparent 100%
  );
}

.lyrics-viewport.static {
  overflow-y: auto;
  mask-image: linear-gradient(
    to bottom,
    transparent 0%,
    #000 4%,
    #000 96%,
    transparent 100%
  );
  -webkit-mask-image: linear-gradient(
    to bottom,
    transparent 0%,
    #000 4%,
    #000 96%,
    transparent 100%
  );
  padding-top: 8px;
  scrollbar-width: thin;
  scrollbar-color: rgba(255, 255, 255, 0.18) transparent;
}

/*
 * Modo pantalla grande: deja visible solo una ventana de letra alrededor de la
 * línea actual, con más contexto hacia adelante que hacia atrás.
 */
.lyrics-viewport.windowed {
  overflow-y: auto;
  padding-right: 9%;
  scrollbar-width: thin;
  scrollbar-color: rgba(255, 255, 255, 0.2) transparent;
  mask-image: linear-gradient(
    to bottom,
    transparent 0%,
    transparent 18%,
    rgba(0, 0, 0, 0.32) 27%,
    #000 40%,
    #000 62%,
    rgba(0, 0, 0, 0.34) 74%,
    transparent 84%,
    transparent 100%
  );
  -webkit-mask-image: linear-gradient(
    to bottom,
    transparent 0%,
    transparent 18%,
    rgba(0, 0, 0, 0.32) 27%,
    #000 40%,
    #000 62%,
    rgba(0, 0, 0, 0.34) 74%,
    transparent 84%,
    transparent 100%
  );
}

.lyrics-viewport.windowed::-webkit-scrollbar {
  width: 7px;
}

.lyrics-viewport.windowed::-webkit-scrollbar-track {
  background: transparent;
}

.lyrics-viewport.windowed::-webkit-scrollbar-thumb {
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.2);
}

.lyrics-viewport.hide-scrollbar {
  scrollbar-width: none;
}

.lyrics-viewport.hide-scrollbar::-webkit-scrollbar {
  display: none;
}

.lyrics-viewport.windowed .line:not(.active) {
  color: rgba(255, 255, 255, 0.4);
}

.lyrics-viewport.windowed .line.past {
  color: rgba(255, 255, 255, 0.19);
}

.lyrics-viewport.windowed .line.active {
  transform: scale(1.085);
  filter:
    drop-shadow(0 0 34px rgba(255, 59, 92, 0.3))
    drop-shadow(0 4px 26px rgba(0, 0, 0, 0.5));
}

.lyrics-viewport.windowed .line.out-of-window {
  opacity: 0.22;
  pointer-events: auto;
}

.lyrics-viewport.windowed .lyrics-track {
  padding: 70vh 0 70vh;
}

.lyrics-viewport.windowed .line.active span {
  background-image: linear-gradient(
    90deg,
    #ffffff 0%,
    #ffffff var(--p, 0%),
    rgba(255, 255, 255, 0.66) var(--p, 0%),
    rgba(255, 255, 255, 0.66) 100%
  );
}

.lyrics-track {
  position: relative;
  padding: 34vh 0;
  transition: transform 0.55s cubic-bezier(0.22, 0.8, 0.28, 1);
  will-change: transform;
}

.lyrics-viewport.static .lyrics-track {
  padding: 8px 0 60px;
}

.line {
  margin: 0 0 var(--lyric-gap, 20px);
  font-size: var(--lyric-size, clamp(20px, 2.05vw, 30px));
  font-weight: 700;
  line-height: 1.34;
  letter-spacing: -0.025em;
  color: rgba(255, 255, 255, 0.5);
  transition:
    color 0.35s ease,
    opacity 0.35s ease,
    transform 0.35s ease,
    filter 0.35s ease;
  cursor: default;
  transform-origin: left center;
}

.line.clickable {
  cursor: pointer;
}

.line.clickable:hover {
  color: rgba(255, 255, 255, 0.78);
}

.line.past {
  color: rgba(255, 255, 255, 0.26);
}

.line.out-of-window {
  opacity: 0;
  pointer-events: none;
}

.line.active {
  color: #fff;
  transform: scale(1.075);
  filter:
    drop-shadow(0 0 28px rgba(255, 59, 92, 0.24))
    drop-shadow(0 3px 12px rgba(0, 0, 0, 0.28));
}

.line.active span {
  background-image: linear-gradient(
    90deg,
    #ffffff 0%,
    #ffffff var(--p, 0%),
    rgba(255, 255, 255, 0.46) var(--p, 0%),
    rgba(255, 255, 255, 0.46) 100%
  );
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
}

.line span {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

/*
 * Modo "línea completa": se ilumina con el color (que sí tiene transición
 * suave) en lugar del degradado de relleno, que no es transicionable y
 * provocaba un salto brusco al cambiar de línea.
 */
.lyrics-shell.full-highlight .line.active span {
  background-image: none;
  -webkit-text-fill-color: currentColor;
}
</style>
