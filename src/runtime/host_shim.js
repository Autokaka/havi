(function () {
  if (window.__havi_step) return;

  var W = document.getElementById('stego').width;
  var STEGO_BITS = 32;
  var canvas = document.getElementById('stego');
  var ctx = canvas.getContext('2d', { willReadFrequently: true });

  function drawStego(ms) {
    var data = ctx.createImageData(W, 1);
    var u = Math.floor(ms) >>> 0;
    for (var i = 0; i < STEGO_BITS; i++) {
      var bit = (u >>> (STEGO_BITS - 1 - i)) & 1;
      var v = bit ? 255 : 0;
      var idx = i * 4;
      data.data[idx] = v;
      data.data[idx + 1] = v;
      data.data[idx + 2] = v;
      data.data[idx + 3] = 255;
    }
    for (var k = STEGO_BITS; k < W; k++) {
      data.data[k * 4 + 3] = 255;
    }
    ctx.putImageData(data, 0, 0);
  }

  window.__havi_step = function (ms) {
    drawStego(ms);
    return ms;
  };

  drawStego(0);
})();
