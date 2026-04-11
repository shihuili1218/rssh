use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Instant;

use crate::error::AppResult;
use crate::models::CastHeader;

/// asciicast v2 录制器。
pub struct Recorder {
    writer: BufWriter<File>,
    start: Instant,
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
            .map_err(|e| crate::error::AppError::Other(e.to_string()))?;
        writeln!(writer, "{header_json}")?;

        Ok(Self {
            writer,
            start: Instant::now(),
        })
    }

    /// 记录一个输出事件。
    pub fn record(&mut self, data: &str) -> AppResult<()> {
        let elapsed = self.start.elapsed().as_secs_f64();
        let event = serde_json::json!([elapsed, "o", data]);
        writeln!(self.writer, "{event}")?;
        Ok(())
    }

    /// 刷新并关闭录制。
    pub fn finish(mut self) -> AppResult<()> {
        self.writer.flush()?;
        Ok(())
    }
}
