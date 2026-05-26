use crate::{api, ipc};

#[napi(object)]
pub struct RenderResult {
    pub frames: u32,
    pub width: i32,
    pub height: i32,
    pub fps: u32,
    pub out: String,
    pub elapsed_ms: u32,
}

#[napi]
pub async fn render(opts: api::RenderOpts) -> napi::Result<RenderResult> {
    let mut handle = api::spawn(opts).map_err(|e| napi::Error::from_reason(e.to_string()))?;
    while let Some(msg) = handle.recv() {
        match msg {
            ipc::Msg::Done { frames, width, height, fps, out, elapsed_ms } => {
                return Ok(RenderResult {
                    frames, width, height, fps, out,
                    elapsed_ms: elapsed_ms.min(u32::MAX as u64) as u32,
                });
            }
            ipc::Msg::Error { message } => return Err(napi::Error::from_reason(message)),
            _ => {}
        }
    }
    Err(napi::Error::from_reason("render exited without done"))
}
