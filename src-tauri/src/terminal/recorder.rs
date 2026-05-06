use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Instant;

use crate::error::AppResult;
use crate::models::CastHeader;

/// asciicast v2 录制器。
///
/// PTY/SSH 把字节流按内核缓冲随便切，多字节 UTF-8 字符可能横跨 chunk。
/// 直接 `from_utf8_lossy` 会把半个字符替换成 U+FFFD，导致 top/less 这类
/// 输出 box-drawing 字符的全屏命令回放时乱码。
/// 这里用 `pending` 缓冲尾部不完整字节，下次拼回去再写。
pub struct Recorder {
    writer: BufWriter<File>,
    start: Instant,
    pending: Vec<u8>,
}

impl Recorder {
    pub fn new(path: PathBuf, cols: u32, rows: u32) -> AppResult<Self> {
        let file = File::create(&path)?;
        let mut writer = BufWriter::new(file);

        let header = CastHeader {
            version: 2,
            width: cols,
            height: rows,
            timestamp: chrono::Utc::now().timestamp(),
        };
        let header_json = serde_json::to_string(&header)
            .map_err(|e| crate::error::AppError::other("recorder_init_failed", serde_json::json!({ "err": e.to_string() })))?;
        writeln!(writer, "{header_json}")?;

        Ok(Self {
            writer,
            start: Instant::now(),
            pending: Vec::new(),
        })
    }

    /// 记录一个输出事件（原始字节）。
    pub fn record(&mut self, data: &[u8]) -> AppResult<()> {
        self.pending.extend_from_slice(data);
        let split = match std::str::from_utf8(&self.pending) {
            Ok(_) => self.pending.len(),
            Err(e) => match e.error_len() {
                // 尾部多字节字符被截断：留到下次再拼。
                None => e.valid_up_to(),
                // 真正的非法字节：整段 lossy 写出，让 U+FFFD 出现在该出现的位置。
                Some(_) => self.pending.len(),
            },
        };
        if split == 0 {
            return Ok(());
        }

        let elapsed = self.start.elapsed().as_secs_f64();
        let chunk = String::from_utf8_lossy(&self.pending[..split]);
        let event = serde_json::json!([elapsed, "o", chunk.as_ref()]);
        writeln!(self.writer, "{event}")?;
        self.pending.drain(..split);
        Ok(())
    }

    /// 刷新并关闭录制。残留的 pending 字节按 lossy 写出。
    pub fn finish(mut self) -> AppResult<()> {
        if !self.pending.is_empty() {
            let elapsed = self.start.elapsed().as_secs_f64();
            let chunk = String::from_utf8_lossy(&self.pending);
            let event = serde_json::json!([elapsed, "o", chunk.as_ref()]);
            writeln!(self.writer, "{event}")?;
            self.pending.clear();
        }
        self.writer.flush()?;
        Ok(())
    }
}
