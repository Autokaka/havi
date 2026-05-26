(function () {
  document.addEventListener('DOMContentLoaded', function () {
    try { fetch('havi-frame://dom-ready', { keepalive: true }); } catch (_) {}
  }, { once: true });
})();
