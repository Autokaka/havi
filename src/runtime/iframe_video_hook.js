// Transparent shim. Replaces any <video src=...> with a style-mirrored
// overlay canvas backed by frames from havi-frame://.

(function () {
  if (window.self === window.top) return;
  if (window.__havi_video) return;

  var SCHEME = 'havi-frame://';
  var HVE = HTMLVideoElement.prototype;
  var MEDIA = HTMLMediaElement.prototype;
  var origPause = HVE.pause;
  var ctDesc = Object.getOwnPropertyDescriptor(HVE, 'currentTime') || Object.getOwnPropertyDescriptor(MEDIA, 'currentTime');
  var pausedDesc = Object.getOwnPropertyDescriptor(HVE, 'paused') || Object.getOwnPropertyDescriptor(MEDIA, 'paused');
  var endedDesc = Object.getOwnPropertyDescriptor(HVE, 'ended') || Object.getOwnPropertyDescriptor(MEDIA, 'ended');
  var sessions = new WeakMap();
  var attaching = new WeakMap();
  var currMs = 0;
  var lastMs = 0;

  var MIRROR_PROPS = [
    'borderRadius','borderTopLeftRadius','borderTopRightRadius','borderBottomLeftRadius','borderBottomRightRadius',
    'transform','transformOrigin','filter','clipPath','boxShadow','opacity','mixBlendMode','mask',
    'zIndex','position','left','top','right','bottom',
  ];

  function setupCanvas(video) {
    var cv = document.createElement('canvas');
    cv.dataset.haviVideoOverlay = '1';
    cv.style.pointerEvents = 'none';
    video.style.visibility = 'hidden';
    video.muted = true;
    var parent = video.parentNode || document.body;
    parent.insertBefore(cv, video.nextSibling);
    syncOverlay(video, cv);
    return cv;
  }

  function syncOverlay(video, cv) {
    var cs = window.getComputedStyle(video);
    var offsetParent = video.offsetParent || document.body;
    var vr = video.getBoundingClientRect();
    var pr = offsetParent.getBoundingClientRect();
    cv.style.position = 'absolute';
    cv.style.left = (vr.left - pr.left) + 'px';
    cv.style.top = (vr.top - pr.top) + 'px';
    cv.style.width = vr.width + 'px';
    cv.style.height = vr.height + 'px';
    for (var i = 0; i < MIRROR_PROPS.length; i++) {
      var p = MIRROR_PROPS[i];
      if (p === 'position' || p === 'left' || p === 'top' || p === 'right' || p === 'bottom') continue;
      cv.style[p] = cs[p];
    }
    var dpr = window.devicePixelRatio || 1;
    var w = Math.max(1, Math.round(vr.width * dpr));
    var h = Math.max(1, Math.round(vr.height * dpr));
    if (cv.width !== w) cv.width = w;
    if (cv.height !== h) cv.height = h;
  }

  function fitRect(srcW, srcH, dstW, dstH, fit) {
    if (fit === 'none') return [(dstW - srcW) / 2, (dstH - srcH) / 2, srcW, srcH];
    if (fit === 'contain' || fit === 'scale-down') {
      var sc = Math.min(dstW / srcW, dstH / srcH);
      var k = fit === 'scale-down' ? Math.min(1, sc) : sc;
      var ws = k * srcW, hs = k * srcH;
      return [(dstW - ws) / 2, (dstH - hs) / 2, ws, hs];
    }
    if (fit === 'cover') {
      var sc2 = Math.max(dstW / srcW, dstH / srcH);
      var ws2 = sc2 * srcW, hs2 = sc2 * srcH;
      return [(dstW - ws2) / 2, (dstH - hs2) / 2, ws2, hs2];
    }
    return [0, 0, dstW, dstH];
  }

  function attach(video) {
    if (sessions.has(video)) return Promise.resolve(sessions.get(video));
    var pending = attaching.get(video);
    if (pending) return pending;
    var src = video.src || video.currentSrc;
    if (!src) return Promise.resolve(null);
    if (/^(blob:|data:|mediastream:)/i.test(src)) return Promise.resolve(null);
    // Sync hide + overlay canvas BEFORE any paint so first frame can't blink real video.
    var cv = setupCanvas(video);
    var birthMs = currMs;
    var shouldPlay = video.autoplay || video.__havi_play_intent;
    var state = {
      attachedSrc: src,
      meta: null, cv: cv, ctx: cv.getContext('2d'),
      paused: !shouldPlay,
      currentTime: 0, ended: false, lastDrawnIdx: -1, failures: 0, dead: false,
      objectFit: window.getComputedStyle(video).objectFit || 'fill',
    };
    sessions.set(video, state);
    var p = doAttach(video, state, src, birthMs).finally(function () { attaching.delete(video); });
    attaching.set(video, p);
    return p;
  }

  var AHEAD = 10;
  var caches = new Map(); // id → { bitmaps, inFlight, readers: WeakMap<state, idx> via Map }

  function getCache(id) {
    var c = caches.get(id);
    if (!c) { c = { bitmaps: new Map(), inFlight: new Set(), readers: new Map() }; caches.set(id, c); }
    return c;
  }

  function ensurePrefetched(state, fromIdx, count) {
    var c = getCache(state.meta.id);
    for (var i = fromIdx; i < fromIdx + count; i++) {
      if (i < 1) continue;
      if (c.bitmaps.has(i) || c.inFlight.has(i)) continue;
      (function (idx) {
        c.inFlight.add(idx);
        fetch(SCHEME + 'frame?id=' + state.meta.id + '&idx=' + idx)
          .then(function (r) { return r.ok ? r.blob() : null; })
          .then(function (b) { return b ? createImageBitmap(b) : null; })
          .then(function (bm) { c.inFlight.delete(idx); if (bm) c.bitmaps.set(idx, bm); })
          .catch(function () { c.inFlight.delete(idx); });
      })(i);
    }
  }

  function evictOld(c) {
    var minIdx = Infinity;
    c.readers.forEach(function (idx) { if (idx < minIdx) minIdx = idx; });
    if (!isFinite(minIdx)) return;
    var floor = minIdx - 2;
    c.bitmaps.forEach(function (bm, idx) {
      if (idx < floor) { bm.close(); c.bitmaps.delete(idx); }
    });
  }

  function doAttach(video, state, src, birthMs) {
    var fps = parseInt(video.dataset.haviFps || '30', 10);
    return fetch(SCHEME + 'open?src=' + encodeURIComponent(src) + '&fps=' + fps)
      .then(function (res) { return res.ok ? res.json() : null; })
      .then(function (meta) {
        if (!meta) { state.dead = true; return null; }
        state.meta = meta;
        state.currentTime = state.paused ? 0 : Math.max(0, (currMs - birthMs) / 1000);
        Object.defineProperty(video, 'videoWidth', { value: meta.width, configurable: true });
        Object.defineProperty(video, 'videoHeight', { value: meta.height, configurable: true });
        Object.defineProperty(video, 'duration', { value: meta.duration, configurable: true });
        Object.defineProperty(video, 'readyState', { value: 4, configurable: true });
        ['loadstart','durationchange','loadedmetadata','loadeddata','canplay','canplaythrough'].forEach(function (e) {
          video.dispatchEvent(new Event(e));
        });
        if (!state.paused) {
          video.dispatchEvent(new Event('play'));
          video.dispatchEvent(new Event('playing'));
        }
        ensurePrefetched(state, 1, AHEAD);
        return state;
      });
  }

  function detach(video) {
    var state = sessions.get(video);
    if (!state) return;
    if (state.meta) {
      var c = caches.get(state.meta.id);
      if (c) {
        c.readers.delete(state);
        if (c.readers.size === 0) {
          c.bitmaps.forEach(function (bm) { bm.close(); });
          caches.delete(state.meta.id);
          fetch(SCHEME + 'close?id=' + state.meta.id, { keepalive: true });
        }
      }
    }
    state.cv.remove();
    sessions.delete(video);
  }

  function resume(video, state) {
    if (!state.paused && !state.ended) return;
    state.paused = false;
    state.ended = false;
    video.dispatchEvent(new Event('play'));
    video.dispatchEvent(new Event('playing'));
  }

  HVE.play = function () {
    var video = this;
    video.__havi_play_intent = true;
    var state = sessions.get(video);
    if (!state) return attach(video).then(function (s) { if (s) resume(video, s); });
    resume(video, state);
    return Promise.resolve();
  };
  HVE.pause = function () {
    this.__havi_play_intent = false;
    var state = sessions.get(this);
    if (!state) { origPause.call(this); return; }
    if (!state.paused) { state.paused = true; this.dispatchEvent(new Event('pause')); }
  };
  Object.defineProperty(HVE, 'paused', {
    configurable: true,
    get: function () { var s = sessions.get(this); return s ? s.paused : pausedDesc.get.call(this); },
  });
  Object.defineProperty(HVE, 'ended', {
    configurable: true,
    get: function () { var s = sessions.get(this); return s ? s.ended : endedDesc.get.call(this); },
  });
  if (ctDesc) {
    Object.defineProperty(HVE, 'currentTime', {
      configurable: true,
      get: function () { var s = sessions.get(this); return s ? s.currentTime : ctDesc.get.call(this); },
      set: function (t) {
        var s = sessions.get(this);
        if (!s) { ctDesc.set.call(this, t); return; }
        s.currentTime = Math.max(0, Math.min(t, s.meta.duration));
        if (s.currentTime < s.meta.duration) s.ended = false;
        this.dispatchEvent(new Event('seeking'));
        this.dispatchEvent(new Event('seeked'));
        this.dispatchEvent(new Event('timeupdate'));
      },
    });
  }

  function onSrcChange(video) {
    var newSrc = video.src || video.currentSrc;
    var state = sessions.get(video);
    if (state && state.attachedSrc === newSrc) return;
    if (state) detach(video);
    attach(video);
  }
  function scan(root) {
    if (!root) return;
    if (root.tagName === 'VIDEO') attach(root);
    if (root.querySelectorAll) root.querySelectorAll('video').forEach(function (v) { attach(v); });
  }
  function initialScan() { scan(document.documentElement); }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', initialScan, { once: true });
  } else {
    initialScan();
  }
  new MutationObserver(function (muts) {
    for (var i = 0; i < muts.length; i++) {
      var m = muts[i];
      if (m.type === 'attributes' && m.target instanceof HTMLVideoElement && m.attributeName === 'src') {
        onSrcChange(m.target); continue;
      }
      m.addedNodes.forEach(function (n) { if (n instanceof Element) scan(n); });
      m.removedNodes.forEach(function (n) { if (n instanceof HTMLVideoElement) detach(n); });
    }
  }).observe(document, { subtree: true, childList: true, attributes: true, attributeFilter: ['src'] });


  var STUCK_LIMIT = 300; // ~10s at 30fps before declaring a video dead
  function paint(video, state, idx) {
    var c = getCache(state.meta.id);
    c.readers.set(state, idx);
    var bm = c.bitmaps.get(idx);
    if (!bm) {
      ensurePrefetched(state, idx, AHEAD);
      state.stuckTicks = (state.stuckTicks || 0) + 1;
      if (state.stuckTicks > STUCK_LIMIT) state.dead = true;
      return;
    }
    state.stuckTicks = 0;
    var cv = state.cv;
    var r = fitRect(bm.width, bm.height, cv.width, cv.height, state.objectFit);
    state.ctx.clearRect(0, 0, cv.width, cv.height);
    state.ctx.drawImage(bm, r[0], r[1], r[2], r[3]);
    state.lastDrawnIdx = idx;
    ensurePrefetched(state, idx + 1, AHEAD);
    evictOld(c);
  }

  function advance(timestampMs) {
    lastMs = currMs;
    currMs = timestampMs;
    var dt = Math.max(0, currMs - lastMs) / 1000;
    document.querySelectorAll('video').forEach(function (v) {
      var state = sessions.get(v);
      if (!state || state.dead) return;
      syncOverlay(v, state.cv);
      if (!state.meta) return;
      if (!state.paused && !state.ended) {
        state.currentTime += dt * (v.playbackRate || 1);
        if (state.currentTime >= state.meta.duration) {
          if (v.loop) { state.currentTime = state.currentTime % state.meta.duration; }
          else {
            state.currentTime = state.meta.duration;
            state.paused = true;
            state.ended = true;
            v.dispatchEvent(new Event('ended'));
          }
        } else {
          v.dispatchEvent(new Event('timeupdate'));
        }
      }
      var idx = Math.max(1, Math.round(state.currentTime * state.meta.fps));
      if (idx === state.lastDrawnIdx) return;
      paint(v, state, idx);
    });
  }

  window.__havi_video = { advance: advance };
  window.__havi_advance_videos = advance;
})();
