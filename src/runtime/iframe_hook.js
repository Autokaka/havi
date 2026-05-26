(function () {
  if (window.__havi_tick) return;
  var RealDate = Date;
  var dateOrigin = __HAVI_DATE_ORIGIN__;
  var currMs = 0;
  var timeOffset = 0;
  var rafQueue = [];
  var timers = {};
  var nextId = 1;

  function FakeDate() {
    if (!(this instanceof FakeDate)) {
      return new RealDate(dateOrigin + currMs).toString();
    }
    return arguments.length
      ? new (Function.prototype.bind.apply(RealDate, [null].concat([].slice.call(arguments))))()
      : new RealDate(dateOrigin + currMs + (timeOffset += 0.01));
  }
  FakeDate.prototype = RealDate.prototype;
  FakeDate.now = function () { return dateOrigin + currMs + (timeOffset += 0.01); };
  FakeDate.parse = RealDate.parse;
  FakeDate.UTC = RealDate.UTC;
  window.Date = FakeDate;
  performance.now = function () { return currMs + (timeOffset += 0.01); };

  window.requestAnimationFrame = function (cb) {
    var id = nextId++;
    rafQueue.push({ id: id, cb: cb });
    return id;
  };
  window.cancelAnimationFrame = function (id) {
    rafQueue = rafQueue.filter(function (r) { return r.id !== id; });
  };
  window.setTimeout = function (cb, delay) {
    var id = nextId++;
    timers[id] = { type: 'timeout', cb: cb, delay: delay || 0, next: currMs + (delay || 0) };
    return id;
  };
  window.clearTimeout = function (id) { delete timers[id]; };
  window.setInterval = function (cb, delay) {
    var id = nextId++;
    var d = Math.max(delay || 0, 1);
    timers[id] = { type: 'interval', cb: cb, delay: d, next: currMs + d };
    return id;
  };
  window.clearInterval = function (id) { delete timers[id]; };

  var seed = 0x2545f491;
  Math.random = function () {
    seed = (seed * 1664525 + 1013904223) >>> 0;
    return seed / 0x100000000;
  };

  function safeInvoke(fn, arg) {
    try { fn(arg); } catch (e) { console.error(e); }
  }

  var animState = new WeakMap();

  function syncAnimations(ms) {
    var anims;
    try { anims = document.getAnimations(); } catch (_) { return; }
    for (var i = 0; i < anims.length; i++) {
      var a = anims[i];
      var state = a.playState;
      if (state === 'idle' || state === 'finished') { animState.delete(a); continue; }
      var s = animState.get(a);
      if (!s) { s = { startMs: ms, pausedAccum: 0, pausedAt: -1, lastSet: null }; animState.set(a, s); }
      if (state === 'paused') {
        if (s.pausedAt < 0) s.pausedAt = ms;
        continue;
      }
      if (s.pausedAt >= 0) {
        s.pausedAccum += ms - s.pausedAt;
        s.pausedAt = -1;
      }
      var rate = typeof a.playbackRate === 'number' ? a.playbackRate : 1;
      if (rate === 0) continue;
      if (s.lastSet !== null && Math.abs(a.currentTime - s.lastSet) > 1) {
        s.startMs = ms - s.pausedAccum - a.currentTime / rate;
      }
      var target = (ms - s.startMs - s.pausedAccum) * rate;
      a.currentTime = target;
      s.lastSet = target;
    }
  }

  window.__havi_tick = {
    process: function (ms) {
      currMs = ms;
      timeOffset = 0;
      var ids = Object.keys(timers);
      for (var i = 0; i < ids.length; i++) {
        var t = timers[ids[i]];
        if (!t) continue;
        while (t.next <= currMs) {
          safeInvoke(t.cb);
          if (t.type === 'timeout') { delete timers[ids[i]]; break; }
          t.next += t.delay;
        }
      }
      var rafs = rafQueue.splice(0);
      for (var j = 0; j < rafs.length; j++) safeInvoke(rafs[j].cb, currMs);
      syncAnimations(ms);
    }
  };
})();
