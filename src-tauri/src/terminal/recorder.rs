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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// 读取 cast 文件并按行解析为 (header_json, event_jsons)。
    fn parse_cast(path: &std::path::Path) -> (Value, Vec<Value>) {
        let body = std::fs::read_to_string(path).unwrap();
        let mut lines = body.lines().filter(|l| !l.is_empty());
        let header: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        let events: Vec<Value> = lines
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .collect();
        (header, events)
    }

    /// 提取所有事件的 stdout 字符串拼接。
    fn cat_outputs(events: &[Value]) -> String {
        events
            .iter()
            .filter(|e| e[1] == "o")
            .map(|e| e[2].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn header_has_v2_shape() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.cast");
        let r = Recorder::new(p.clone(), 80, 24).unwrap();
        r.finish().unwrap();
        let (header, events) = parse_cast(&p);
        assert_eq!(header["version"], 2);
        assert_eq!(header["width"], 80);
        assert_eq!(header["height"], 24);
        assert!(header["timestamp"].is_i64());
        assert!(events.is_empty());
    }

    #[test]
    fn ascii_recorded_inline() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.cast");
        let mut r = Recorder::new(p.clone(), 80, 24).unwrap();
        r.record(b"hello").unwrap();
        r.finish().unwrap();
        let (_h, events) = parse_cast(&p);
        assert_eq!(cat_outputs(&events), "hello");
    }

    #[test]
    fn split_utf8_recovered_across_chunks() {
        // "中" = E4 B8 AD（3 字节），分两次喂入。最终输出必须完整无损。
        // 这个测试**只**看 finish 后的最终状态——BufWriter 中间不刷盘，
        // record 之间读文件不可靠。
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.cast");
        let mut r = Recorder::new(p.clone(), 80, 24).unwrap();
        r.record(&[0xE4, 0xB8]).unwrap();
        r.record(&[0xAD]).unwrap();
        r.finish().unwrap();
        let (_h, events) = parse_cast(&p);
        assert_eq!(cat_outputs(&events), "中");
    }

    #[test]
    fn split_utf8_count_distinguishes_buffering() {
        // 如果实现没缓冲不完整 UTF-8，半中字会立刻 lossy → 多产一个事件。
        // 缓冲版：半中字(无事件) + 余字(1 事件) + 'x'(1 事件) = 2 事件
        // 不缓冲：半中字(1 lossy 事件) + 余字(1 事件) + 'x'(1 事件) = 3 事件
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("d.cast");
        let mut r = Recorder::new(p.clone(), 80, 24).unwrap();
        r.record(&[0xE4, 0xB8]).unwrap();
        r.record(&[0xAD]).unwrap();
        r.record(b"x").unwrap();
        r.finish().unwrap();
        let (_h, events) = parse_cast(&p);
        assert_eq!(events.len(), 2, "events: {events:?}");
        assert_eq!(cat_outputs(&events), "中x");
    }

    #[test]
    fn ascii_then_split_utf8_split_then_complete() {
        // 头有 ASCII + 尾不完整：split=valid_up_to() 让 ASCII 当场写出，
        // 尾巴留到下次。最终拼回完整。
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("e.cast");
        let mut r = Recorder::new(p.clone(), 80, 24).unwrap();
        r.record(b"ab\xE4\xB8").unwrap();
        r.record(&[0xAD]).unwrap();
        r.finish().unwrap();
        let (_h, events) = parse_cast(&p);
        assert_eq!(cat_outputs(&events), "ab中");
    }

    #[test]
    fn truly_invalid_byte_lossy_inline() {
        // 0xFF 是真正非法 UTF-8 起始 — 不应该缓冲，立即 lossy 写出
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("e.cast");
        let mut r = Recorder::new(p.clone(), 80, 24).unwrap();
        r.record(&[0xFF, b'x']).unwrap();
        r.finish().unwrap();
        let (_h, events) = parse_cast(&p);
        let out = cat_outputs(&events);
        assert!(out.contains('\u{FFFD}'));
        assert!(out.contains('x'));
    }

    #[test]
    fn finish_flushes_residual_pending_lossy() {
        // 只写半个多字节字符然后 finish — pending 必须 lossy 出来，不能丢
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.cast");
        let mut r = Recorder::new(p.clone(), 80, 24).unwrap();
        r.record(&[0xE4, 0xB8]).unwrap(); // 半个"中"
        r.finish().unwrap();
        let (_h, events) = parse_cast(&p);
        assert!(!events.is_empty());
        // lossy → U+FFFD
        assert!(cat_outputs(&events).contains('\u{FFFD}'));
    }

    #[test]
    fn event_timestamps_monotonic() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("g.cast");
        let mut r = Recorder::new(p.clone(), 80, 24).unwrap();
        r.record(b"first").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        r.record(b"second").unwrap();
        r.finish().unwrap();
        let (_h, events) = parse_cast(&p);
        let stamps: Vec<f64> = events.iter().map(|e| e[0].as_f64().unwrap()).collect();
        assert!(stamps.len() >= 2);
        for w in stamps.windows(2) {
            assert!(w[1] >= w[0], "timestamps not monotonic: {stamps:?}");
        }
        // 不断言具体 sleep 时长——CI sleep 精度受 OS 调度影响易 flaky，
        // 单调 + 非负就够了（thread::sleep 5ms 只是保证两个事件时间不同）。
        assert!(stamps[0] >= 0.0);
    }
}
