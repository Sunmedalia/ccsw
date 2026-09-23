//! Observe upstream bytes before protocol conversion without modifying the payload.
use super::*;
use crate::usage::{Request, Ticket};

pub(super) async fn begin(
    state: &ServerState,
    target: &RouteTarget,
    id: &str,
    profile: &Profile,
    model: &str,
    kind: &'static str,
) -> Option<Ticket> {
    Ticket::begin(
        state.usage.clone(),
        Request {
            config: target.config_path.clone(),
            client: if target.codex { "Codex" } else { "Claude" },
            provider: id.into(),
            name: profile.name.clone(),
            model: model.into(),
            kind,
            api_format: profile.api_format,
        },
    )
    .await
}

struct Observer {
    ticket: Ticket,
    streaming: bool,
    decoder: Decoder,
    body: Vec<u8>,
    invalid: bool,
    completed: bool,
}
impl Observer {
    fn new(mut ticket: Ticket, streaming: bool) -> Self {
        ticket.outcome = "interrupted";
        Self {
            ticket,
            streaming,
            decoder: Decoder::default(),
            body: vec![],
            invalid: false,
            completed: false,
        }
    }
    fn push(&mut self, bytes: &[u8]) {
        if self.invalid || self.completed {
            return;
        }
        if !self.streaming {
            if self.body.len().saturating_add(bytes.len()) > BODY_LIMIT {
                self.invalid = true;
                self.body.clear();
            } else {
                self.body.extend_from_slice(bytes);
            }
            return;
        }
        let frames = match self.decoder.push(bytes) {
            Ok(v) => v,
            Err(_) => {
                self.invalid = true;
                return;
            }
        };
        for frame in frames {
            if frame == "[DONE]" {
                self.ticket.output_finished();
                self.ticket.outcome = "success";
                self.completed = true;
                break;
            }
            let value: Value = match serde_json::from_str(&frame) {
                Ok(v) => v,
                Err(_) => {
                    self.invalid = true;
                    return;
                }
            };
            self.ticket.observe_usage(&value);
            if value.get("error").is_some_and(|e| !e.is_null())
                || matches!(value["type"].as_str(), Some("error" | "response.failed"))
            {
                self.ticket.outcome = "failed";
                self.completed = true;
                break;
            }
            if matches!(
                value["type"].as_str(),
                Some("message_stop" | "response.completed" | "response.incomplete")
            ) {
                self.ticket.outcome = "success";
                self.ticket.output_finished();
                self.completed = true;
                break;
            }
        }
    }
    fn finish(&mut self) {
        if self.streaming || self.invalid {
            return;
        }
        match serde_json::from_slice::<Value>(&self.body) {
            Ok(v) => {
                self.ticket.observe_usage(&v);
                self.ticket.outcome =
                    if v.get("error").is_some_and(|e| !e.is_null()) || v["status"] == "failed" {
                        "failed"
                    } else {
                        "success"
                    };
            }
            Err(_) => self.ticket.outcome = "failed",
        }
    }
}

pub(super) fn observe(
    response: reqwest::Response,
    ticket: Option<Ticket>,
    streaming: bool,
) -> reqwest::Response {
    let Some(ticket) = ticket else {
        return response;
    };
    // HTTP errors may never be consumed by ingress; finalize them here.
    if !response.status().is_success() {
        drop(ticket);
        return response;
    }
    let mut builder = axum::http::Response::builder()
        .status(response.status())
        .version(response.version());
    *builder.headers_mut().expect("response headers") = response.headers().clone();
    let mut upstream = response.bytes_stream();
    // Own the guard outside the generator so an unpolled response is still finalized.
    let mut observer = Observer::new(ticket, streaming);
    let stream = async_stream::stream! {
        while let Some(chunk) = upstream.next().await {
            match chunk {
                Ok(bytes) => { observer.push(&bytes); yield Ok::<Bytes, reqwest::Error>(bytes); }
                Err(error) => { yield Err(error); return; }
            }
        }
        observer.finish();
    };
    builder
        .body(reqwest::Body::wrap_stream(stream))
        .expect("metered response")
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::tests::{request, settled};

    #[tokio::test]
    async fn stream_duration_includes_wait_before_buffered_output() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(crate::usage::FILE);
        let ticket = Ticket::begin(
            crate::usage::Writer::new(path.clone()),
            request("Claude", "p", "generation"),
        )
        .await
        .unwrap();
        // Headers/hidden reasoning may take most of the time; the final text can
        // arrive in one burst. That wait must remain in the denominator.
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        let mut observer = Observer::new(ticket, true);
        observer
            .push(b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hello\"}}\n\n");
        observer.push(b"data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":20}}\n\ndata: {\"type\":\"message_stop\"}\n\n");
        drop(observer);
        let t = settled(&path, 1)
            .await
            .total(None, None, None, "generation");
        assert_eq!(t.speed_output, 20);
        assert_eq!(t.speed_samples, 1);
        assert!(t.speed_ms >= 40);
    }

    #[tokio::test]
    async fn observes_native_and_translated_streams_without_altering_bytes() {
        for (data, input, output, cache) in [
            (
                "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":11,\"cache_read_input_tokens\":5}}}\r\n\r\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":3}}\n\ndata: {\"type\":\"message_stop\"}\n\n",
                11,
                3,
                5,
            ),
            (
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":4}}\n\ndata: [DONE]\n\n",
                12,
                4,
                0,
            ),
            (
                "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":13,\"output_tokens\":5,\"input_tokens_details\":{\"cached_tokens\":6}}}}\n\n",
                13,
                5,
                6,
            ),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join(crate::usage::FILE);
            let ticket = Ticket::begin(
                crate::usage::Writer::new(path.clone()),
                request("Claude", "p", "generation"),
            )
            .await;
            let chunks = data
                .as_bytes()
                .iter()
                .map(|b| Ok::<_, std::io::Error>(Bytes::copy_from_slice(&[*b])))
                .collect::<Vec<_>>();
            let original: reqwest::Response = axum::http::Response::new(
                reqwest::Body::wrap_stream(futures_util::stream::iter(chunks)),
            )
            .into();
            let observed = observe(original, ticket, true);
            assert_eq!(observed.bytes().await.unwrap(), data.as_bytes());
            let s = settled(&path, 1).await;
            let t = s.total(None, None, None, "generation");
            assert_eq!(
                (t.calls, t.success, t.input, t.output, t.cache_read),
                (1, 1, input, output, cache)
            );
        }
    }

    #[tokio::test]
    async fn failures_disconnects_unknown_usage_and_unpolled_bodies() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(crate::usage::FILE);
        for (body, status, streaming, consume) in [
            ("{}", 429, false, false),
            (
                "data: {\"type\":\"error\",\"error\":{}}\n\n",
                200,
                true,
                true,
            ),
            (
                "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":2}}}\n\n",
                200,
                true,
                true,
            ),
            ("data: [DONE]\n\n", 200, true, false),
            (
                "{\"usage\":{\"input_tokens\":4,\"output_tokens\":6}}",
                200,
                false,
                true,
            ),
            ("{}", 200, false, true),
            ("not json", 200, false, true),
        ] {
            let ticket = Ticket::begin(
                crate::usage::Writer::new(path.clone()),
                request("Codex", "p", "generation"),
            )
            .await;
            let original: reqwest::Response = axum::http::Response::builder()
                .status(status)
                .body(reqwest::Body::from(body))
                .unwrap()
                .into();
            let observed = observe(original, ticket, streaming);
            if consume {
                let _ = observed.bytes().await.unwrap();
            } else {
                drop(observed);
            }
        }
        // A connection failure before headers drops the original ticket.
        drop(
            Ticket::begin(
                crate::usage::Writer::new(path.clone()),
                request("Codex", "p", "generation"),
            )
            .await,
        );
        let s = settled(&path, 8).await;
        let t = s.total(None, None, None, "generation");
        assert_eq!(
            (t.calls, t.success, t.failed, t.interrupted, t.unknown),
            (8, 2, 4, 2, 7)
        );
        assert_eq!((t.input, t.output), (6, 6));
    }
}
