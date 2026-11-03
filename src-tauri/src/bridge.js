/*
 * Puente que se inyecta en la ventana de music.youtube.com.
 * - Lee el estado del reproductor (mediaSession + elemento <video>).
 * - Lo envía al servidor local de la app (para la UI y el control remoto).
 * - Expone window.__ytm para recibir comandos desde Rust.
 */
(function () {
  "use strict";

  if (window.top !== window) return;
  if (window.__ytmBridgeInstalled) return;
  window.__ytmBridgeInstalled = true;

  var PORT = __PORT__;
  var STATE_URL = "http://127.0.0.1:" + PORT + "/api/state";
  var ipcUnavailable = false;
  var trackSignature = "";
  var trackChangedAt = 0;
  var pendingSeek = null;
  var pendingSeekUntil = 0;
  var pendingSeekStartedAt = 0;

  function video() {
    return (
      document.querySelector("#movie_player video.html5-main-video") ||
      document.querySelector("#movie_player video") ||
      document.querySelector("ytmusic-player video") ||
      document.querySelector("video")
    );
  }

  function parseClock(value) {
    var parts = String(value || "")
      .trim()
      .split(":")
      .map(function (part) {
        return parseInt(part, 10);
      });

    if (!parts.length || parts.some(function (part) { return isNaN(part); })) {
      return 0;
    }

    var seconds = 0;
    for (var i = 0; i < parts.length; i += 1) {
      seconds = seconds * 60 + parts[i];
    }
    return seconds;
  }

  function readBarTime() {
    var node = document.querySelector("ytmusic-player-bar .time-info");
    if (!node) return { position: 0, duration: 0 };

    var text = (node.textContent || "").replace(/\s+/g, " ").trim();
    var slash = text.lastIndexOf("/");
    if (slash < 0) return { position: 0, duration: 0 };

    return {
      position: Math.max(0, parseClock(text.slice(0, slash))),
      duration: parseClock(text.slice(slash + 1)),
    };
  }

  function clean(value, max) {
    if (!value) return "";
    var text = String(value).replace(/\s+/g, " ").trim();
    return text.length > max ? text.slice(0, max) : text;
  }

  function readMeta() {
    var title = "";
    var artist = "";
    var album = "";
    var artwork = "";

    try {
      var md = navigator.mediaSession && navigator.mediaSession.metadata;
      if (md) {
        title = md.title || "";
        artist = md.artist || "";
        album = md.album || "";
        if (md.artwork && md.artwork.length) {
          artwork = md.artwork[md.artwork.length - 1].src || "";
        }
      }
    } catch (error) {
      /* la API de media session puede no estar disponible */
    }

    var bar = document.querySelector("ytmusic-player-bar");
    if (bar) {
      if (!title) {
        var titleEl = bar.querySelector("yt-formatted-string.title");
        if (titleEl) title = titleEl.textContent || "";
      }

      if (!artist) {
        var bylineEl = bar.querySelector("yt-formatted-string.byline");
        var raw = bylineEl && bylineEl.textContent ? bylineEl.textContent : "";
        var parts = raw
          .split("\u2022")
          .map(function (part) {
            return part.trim();
          })
          .filter(Boolean);
        if (parts.length) artist = parts[0];
        if (!album && parts.length > 1) album = parts[1];
      }

      if (!artwork) {
        var img = bar.querySelector(".image img") || bar.querySelector("img");
        if (img) artwork = img.src || "";
      }
    }

    return {
      title: clean(title, 300),
      artist: clean(artist, 300),
      album: clean(album, 300),
      artwork: clean(artwork, 2000),
    };
  }

  function getVideoId() {
    try {
      var id = new URL(location.href).searchParams.get("v");
      if (id) return id;
    } catch (error) {
      /* ignorar */
    }

    var link = document.querySelector('ytmusic-player-bar a[href*="watch?v="]');
    if (link) {
      try {
        return new URL(link.href).searchParams.get("v") || "";
      } catch (error) {
        /* ignorar */
      }
    }
    return "";
  }

  function readNextTrack(currentVideoId) {
    try {
      var items = document.querySelectorAll(
        "ytmusic-player-queue-item, ytmusic-player-queue-item-renderer"
      );

      for (var i = 0; i < items.length; i += 1) {
        var item = items[i];
        var link = item.querySelector('a[href*="watch?v="]');
        var itemId = "";
        if (link) {
          try {
            itemId = new URL(link.href, location.href).searchParams.get("v") || "";
          } catch (error) {
            itemId = "";
          }
        }
        if (itemId && currentVideoId && itemId === currentVideoId) continue;

        var titleEl = item.querySelector(
          ".song-title, .title, yt-formatted-string.title"
        );
        var bylineEl = item.querySelector(".byline");
        var title = clean(titleEl && titleEl.textContent, 300);
        var byline = clean(bylineEl && bylineEl.textContent, 600);
        if (!title) continue;

        var parts = byline
          .split("\u2022")
          .map(function (part) { return part.trim(); })
          .filter(Boolean);

        return {
          title: title,
          artist: parts[0] || "",
          album: parts[1] || "",
          duration: 0,
        };
      }
    } catch (error) {
      /* La cola puede cambiar mientras YouTube actualiza el DOM. */
    }

    return null;
  }

  function readLikeStatus() {
    var selectors = [
      "ytmusic-player-bar ytmusic-like-button-renderer #like-button",
      "ytmusic-player-bar ytmusic-like-button-renderer button[aria-label*='Like']",
      "ytmusic-player-bar button[aria-label='Like']",
      "ytmusic-player-bar button[aria-label='Me gusta']",
    ];

    for (var index = 0; index < selectors.length; index += 1) {
      var button = document.querySelector(selectors[index]);
      if (!button) continue;

      var pressed = button.getAttribute("aria-pressed");
      if (pressed === "true") return true;
      if (pressed === "false") return false;

      var label = (
        button.getAttribute("aria-label") ||
        button.getAttribute("title") ||
        ""
      ).toLowerCase();
      if (label.indexOf("remove like") >= 0 || label.indexOf("quitar me gusta") >= 0) {
        return true;
      }
      return false;
    }

    return false;
  }

  function readState() {
    var meta = readMeta();
    var el = video();
    var playing = false;
    var videoDuration = el && isFinite(el.duration) ? el.duration : 0;
    var bar = readBarTime();
    var barDuration = bar.duration;
    var barPosition = bar.position;
    var duration = videoDuration;
    var position = el && isFinite(el.currentTime) ? el.currentTime : 0;
    var videoMatchesBar =
      videoDuration <= 0 ||
      barDuration <= 0 ||
      Math.abs(videoDuration - barDuration) <= 3;

    if (barDuration > 0) {
      // La barra oficial es la base. El video solo se usa como refinamiento
      // cuando está dentro de una ventana coherente con la barra; si no,
      // mandan los valores de la barra.
      duration = barDuration;

      if (
        !videoMatchesBar ||
        position < 0 ||
        !isFinite(position) ||
        Math.abs(position - barPosition) > 3
      ) {
        position = Math.max(0, Math.min(barPosition, barDuration));
      }
    }

    if (el) {
      playing = !el.paused && !el.ended;
    } else {
      try {
        playing = navigator.mediaSession.playbackState === "playing";
      } catch (error) {
        playing = false;
      }
    }

    var nextSignature = [meta.title, meta.artist, meta.album].join("|");
    if (nextSignature && nextSignature !== trackSignature) {
      trackSignature = nextSignature;
      trackChangedAt = Date.now();
    }

    var currentVideoId = getVideoId();
    return {
      title: meta.title,
      artist: meta.artist,
      album: meta.album,
      artwork: meta.artwork,
      video_id: currentVideoId,
      duration: duration,
      position: position,
      playing: playing,
      volume: el && isFinite(el.volume) ? el.volume : 1,
      liked: readLikeStatus(),
      next_track: readNextTrack(currentVideoId),
    };
  }

  function sendState() {
    try {
      if (location.hostname !== "music.youtube.com") return;

      var state = readState();
      var internals = window.__TAURI_INTERNALS__;

      if (!ipcUnavailable && internals && typeof internals.invoke === "function") {
        internals
          .invoke("plugin:event|emit", {
            event: "bridge-state",
            payload: state,
          })
          .catch(function () {
            ipcUnavailable = true;
            sendHttp(state);
          });
        return;
      }

      sendHttp(state);
    } catch (error) {
      /* nunca romper la página de YouTube Music */
    }
  }

  function sendHttp(state) {
    try {
      /* no-cors + text/plain evita preflight CORS; el cuerpo se envía igual. */
      fetch(STATE_URL, {
        method: "POST",
        mode: "no-cors",
        cache: "no-store",
        headers: { "Content-Type": "text/plain" },
        body: JSON.stringify(state),
        keepalive: true,
      }).catch(function () {
        /* el servidor puede no estar listo todavía */
      });
    } catch (error) {
      /* ignorar */
    }
  }

  function clickAny(selectors) {
    for (var i = 0; i < selectors.length; i += 1) {
      var el = document.querySelector(selectors[i]);
      if (el && typeof el.click === "function") {
        el.click();
        return true;
      }
    }
    return false;
  }

  function pressKey(key, shiftKey) {
    var event = new KeyboardEvent("keydown", {
      key: key,
      code: key,
      bubbles: true,
      cancelable: true,
      shiftKey: !!shiftKey,
    });
    (document.body || document).dispatchEvent(event);
  }

  window.__ytm = {
    like: function () {
      if (readLikeStatus()) return;
      clickAny([
        "ytmusic-player-bar ytmusic-like-button-renderer #like-button",
        "ytmusic-player-bar ytmusic-like-button-renderer button[aria-label*='Like']",
        "ytmusic-player-bar button[aria-label='Like']",
        "ytmusic-player-bar button[aria-label='Me gusta']",
      ]);
      window.setTimeout(sendState, 250);
    },
    play: function () {
      var el = video();
      if (el) {
        var promise = el.play();
        if (promise && promise.catch) promise.catch(function () {});
        return;
      }
      clickAny(["ytmusic-player-bar #play-pause-button", "#play-pause-button"]);
    },

    pause: function () {
      var el = video();
      if (el) {
        el.pause();
        return;
      }
      clickAny(["ytmusic-player-bar #play-pause-button", "#play-pause-button"]);
    },

    toggle: function () {
      var el = video();
      if (el) {
        if (el.paused) {
          var promise = el.play();
          if (promise && promise.catch) promise.catch(function () {});
        } else {
          el.pause();
        }
        return;
      }
      clickAny(["ytmusic-player-bar #play-pause-button", "#play-pause-button"]);
    },

    next: function () {
      var clicked = clickAny([
        "ytmusic-player-bar #next-button",
        "ytmusic-player-bar .next-button",
        "#next-button",
        ".next-button",
        'ytmusic-player-bar [aria-label="Next song"]',
        '[aria-label="Next song"]',
      ]);
      if (!clicked) pressKey("N", true);
    },

    prev: function () {
      var clicked = clickAny([
        "ytmusic-player-bar #previous-button",
        "ytmusic-player-bar .previous-button",
        "#previous-button",
        ".previous-button",
        'ytmusic-player-bar [aria-label="Previous song"]',
        '[aria-label="Previous song"]',
      ]);
      if (!clicked) pressKey("P", true);
    },

    seek: function (seconds) {
      var el = video();
      if (!el) return;
      var state = readState();
      var durationReady =
        !state.duration ||
        !isFinite(el.duration) ||
        Math.abs(el.duration - state.duration) <= 3;
      var trackReady =
        !trackChangedAt || Date.now() - trackChangedAt >= 1200;

      // Durante el cambio automático la UI puede recibir los metadatos nuevos
      // antes de que YouTube reemplace el elemento de vídeo. No debemos buscar
      // en ese elemento porque todavía reproduce la canción anterior.
      if (!durationReady || !trackReady) {
        pendingSeek = seconds;
        pendingSeekUntil = Date.now() + 3000;
        pendingSeekStartedAt = Date.now();
        return;
      }

      var max = state.duration > 0 ? state.duration : el.duration;
      el.currentTime = Math.max(0, Math.min(seconds, max));
      sendState();
    },

    volume: function (value) {
      var el = video();
      if (!el) return;
      el.volume = Math.max(0, Math.min(1, value));
      el.muted = false;
      sendState();
    },

    readState: readState,

    sendNow: sendState,
  };

  window.setInterval(function () {
    if (pendingSeek === null) return;
    if (Date.now() > pendingSeekUntil) {
      pendingSeek = null;
      pendingSeekStartedAt = 0;
      return;
    }

    var value = pendingSeek;
    var el = video();
    var state = readState();
    var durationReady =
      el &&
      (!state.duration ||
        !isFinite(el.duration) ||
        Math.abs(el.duration - state.duration) <= 3);
    var trackReady =
      !trackChangedAt || Date.now() - trackChangedAt >= 1200;

    if (!el || !trackReady) return;
    if (!durationReady && Date.now() - pendingSeekStartedAt < 1800) return;

    pendingSeek = null;
    var max = state.duration > 0 ? state.duration : el.duration;
    el.currentTime = Math.max(0, Math.min(value, max));
    pendingSeekStartedAt = 0;
    sendState();
  }, 150);

  function forceDarkTheme() {
    if (location.hostname !== "music.youtube.com") return;
    var root = document.documentElement;
    if (!root) return;
    if (!root.hasAttribute("dark")) root.setAttribute("dark", "");
    if (!root.hasAttribute("darker-dark-theme")) {
      root.setAttribute("darker-dark-theme", "");
    }
    root.style.colorScheme = "dark";
  }

  forceDarkTheme();
  window.setInterval(forceDarkTheme, 1500);

  window.setInterval(sendState, 500);
  window.setTimeout(sendState, 900);
})();
