// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use std::path::{Path, PathBuf};

pub const STEGO_BITS: u32 = 32;

pub fn write_host(target_url: &str, width: u32, height: u32) -> std::io::Result<String> {
    let normalized = normalize_source(target_url);
    let html = build_host_html(&normalized, width, height);
    let path: PathBuf = crate::scratch_dir().join("host.html");
    std::fs::write(&path, html)?;
    Ok(format!("file://{}", abs(&path)))
}

fn normalize_source(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("file:") {
        return format!("file:///{}", rest.trim_start_matches('/'));
    }
    if s.starts_with("data:") || s.contains("://") { return s.to_string(); }
    let p = Path::new(s);
    let abs = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    format!("file:///{}", abs.to_string_lossy().trim_start_matches('/'))
}

fn abs(p: &Path) -> String {
    p.canonicalize()
        .unwrap_or_else(|_| p.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn build_host_html(target_url: &str, width: u32, height: u32) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="UTF-8">
  <style>
    * {{ margin: 0; padding: 0; box-sizing: border-box; }}
    html, body {{ width: {w}px; height: {h_plus_one}px; overflow: hidden; background: transparent; }}
    #target {{
      position: absolute; top: 0; left: 0;
      width: {w}px; height: {h}px;
      border: none; display: block;
    }}
    #stego {{
      position: absolute; top: {h}px; left: 0;
      width: {w}px; height: 1px;
      display: block;
      image-rendering: pixelated;
    }}
    @keyframes havi_heartbeat {{
      0% {{ opacity: 0.5; }}
      100% {{ opacity: 1; }}
    }}
    #heartbeat {{
      position: absolute; bottom: 0; right: 0;
      width: 1px; height: 1px;
      background: #000;
      animation: havi_heartbeat 1s linear infinite;
      pointer-events: none;
    }}
  </style>
</head>
<body>
  <iframe id="target" name="havi_target" src="{url}"></iframe>
  <canvas id="stego" width="{w}" height="1"></canvas>
  <div id="heartbeat"></div>
  <script>{shim}</script>
</body>
</html>
"#,
        w = width,
        h = height,
        h_plus_one = height + 1,
        url = target_url,
        shim = HOST_SHIM,
    )
}

const HOST_SHIM: &str = include_str!("../runtime/host_shim.js");

pub fn decode_stego(buf: &[u8], width: i32, height: i32) -> Option<u32> {
    let w = usize::try_from(width).ok()?;
    let h = usize::try_from(height).ok()?;
    if w < STEGO_BITS as usize || h < 2 { return None; }
    let stride = w.checked_mul(4)?;
    let row_start = (h - 1).checked_mul(stride)?;
    let mut ts: u32 = 0;
    for i in 0..STEGO_BITS as usize {
        let r = buf[row_start + i * 4];
        let bit = if r > 127 { 1 } else { 0 };
        ts = (ts << 1) | bit;
    }
    Some(ts)
}
