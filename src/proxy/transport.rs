use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use std::time::Duration;

pub const HEADER_TIMEOUT: Duration = Duration::from_secs(120);
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(180);
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(600);
pub const BODY_LIMIT: usize = 32 * 1024 * 1024;
pub const EVENT_LIMIT: usize = 1024 * 1024;
pub const ERROR_LIMIT: usize = 16 * 1024;

#[derive(Default)]
pub struct Decoder {
    pending: Vec<u8>,
}
impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>> {
        let mut events = Vec::new();
        for byte in bytes {
            self.pending.push(*byte);
            if self.pending.len() > EVENT_LIMIT {
                bail!("upstream SSE event exceeds 1 MiB");
            }
            let separator = if self.pending.ends_with(b"\r\n\r\n") {
                4
            } else if self.pending.ends_with(b"\n\r\n") {
                3
            } else if self.pending.ends_with(b"\n\n") {
                2
            } else {
                continue;
            };
            let frame = std::str::from_utf8(&self.pending[..self.pending.len() - separator])
                .context("invalid UTF-8 in upstream SSE")?;
            let data = frame
                .lines()
                .filter_map(|line| {
                    line.strip_prefix("data:")
                        .map(|s| s.strip_prefix(' ').unwrap_or(s))
                })
                .collect::<Vec<_>>()
                .join("\n");
            if !data.is_empty() {
                events.push(data);
            }
            self.pending.clear();
        }
        Ok(events)
    }
}

pub async fn read_body(
    response: reqwest::Response,
    limit: usize,
    truncate: bool,
) -> Result<Vec<u8>> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        let remaining = limit.saturating_sub(bytes.len());
        if chunk.len() > remaining {
            if truncate {
                bytes.extend_from_slice(&chunk[..remaining]);
                return Ok(bytes);
            }
            bail!("upstream response exceeds {} bytes", limit);
        }
        bytes.extend_from_slice(&chunk);
        if truncate && bytes.len() == limit {
            break;
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_split_at_every_byte_and_mixed_delimiters() {
        let input = "data: 中文😀\r\n\r\n: heartbeat\n\ndata: one\ndata: two\n\r\n";
        for split in 0..=input.len() {
            let mut decoder = Decoder::default();
            let mut out = decoder.push(&input.as_bytes()[..split]).unwrap();
            out.extend(decoder.push(&input.as_bytes()[split..]).unwrap());
            assert_eq!(out, vec!["中文😀", "one\ntwo"]);
        }
    }
    #[test]
    fn reject_unbounded_or_invalid_events() {
        assert!(
            Decoder::default()
                .push(&vec![b'x'; EVENT_LIMIT + 1])
                .is_err()
        );
        assert!(Decoder::default().push(b"data: \xff\n\n").is_err());
    }
}
