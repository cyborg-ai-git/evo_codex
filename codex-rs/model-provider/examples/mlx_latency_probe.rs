//! Measure local SSE latency separately from server prefill and decode throughput.
//! Usage: mlx_latency_probe BASE_URL MODEL chat|responses REPETITIONS RUNS [REQUEST_JSON]
use codex_http_client::ClientRouteClass;
use codex_http_client::HttpClientFactory;
use codex_http_client::OutboundProxyPolicy;
use codex_http_client::RouteAwareClientPool;
use serde_json::Value;
use serde_json::json;
use std::error::Error;
use std::time::Duration;
use std::time::Instant;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 6 {
        return Err("Usage: mlx_latency_probe BASE_URL MODEL chat|responses REPETITIONS RUNS [REQUEST_JSON]".into());
    }
    let base = args[1].trim_end_matches('/');
    let url = url::Url::parse(base)?;
    if url.scheme() != "http"
        || !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
    {
        return Err("This benchmark only accepts a local HTTP endpoint".into());
    }
    let repetitions: usize = args[4].parse()?;
    let runs: usize = args[5].parse()?;
    let text = format!(
        "{}\nWrite the numbers from 1 through 100, separated by spaces. Do not use tools.",
        "Reference material: the local service processes requests in sequence and returns a stream of text to the client.\n".repeat(repetitions)
    );
    let chat = args[3] == "chat";
    let payload = if let Some(path) = args.get(6) {
        serde_json::from_slice::<Value>(&std::fs::read(path)?)?
    } else if chat {
        json!({"model":args[2],"messages":[{"role":"user","content":text}],"stream":true,
            "stream_options":{"include_usage":true},"max_tokens":96,"temperature":0.0,"enable_thinking":false})
    } else {
        json!({"model":args[2],"input":[{"role":"user","content":text}],"stream":true,
            "max_output_tokens":96,"temperature":0.0,"enable_thinking":false})
    };
    let client = RouteAwareClientPool::new(
        HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
        ClientRouteClass::Other,
    );
    for run in 1..=runs {
        let metrics = client
            .get(format!("{base}/metrics"))
            .send()
            .await?
            .json::<Value>()
            .await?;
        if metrics["summary"]["in_flight"].as_u64().unwrap_or(1) != 0 {
            return Err("The server is busy; stop other generations before benchmarking".into());
        }
        let endpoint = if chat {
            "chat/completions"
        } else {
            "responses"
        };
        let start = Instant::now();
        let mut response = client
            .post(format!("{base}/v1/{endpoint}"))
            .json(&payload)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}: {}", response.status(), response.text().await?).into());
        }
        let headers_s = start.elapsed().as_secs_f64();
        let mut pending = Vec::new();
        let mut first_event = None;
        let mut first_delta = None;
        let mut last_delta = None;
        let mut delta_events = 0;
        let mut output_bytes = 0;
        let mut usage = Value::Null;
        let mut completed = false;
        while let Some(chunk) =
            tokio::time::timeout(Duration::from_secs(300), response.chunk()).await??
        {
            pending.extend_from_slice(&chunk);
            if pending.len() > 4 * 1024 * 1024 {
                return Err("SSE event exceeds the benchmark buffer limit".into());
            }
            while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = pending.drain(..=end).collect();
                let line = std::str::from_utf8(&line)?.trim();
                let Some(data) = line.strip_prefix("data:").map(str::trim) else {
                    continue;
                };
                let elapsed = start.elapsed().as_secs_f64();
                first_event.get_or_insert(elapsed);
                if data == "[DONE]" {
                    completed = true;
                    continue;
                }
                let event: Value = serde_json::from_str(data)?;
                if event.get("error").is_some() || event["type"] == "response.failed" {
                    return Err(format!("Stream failed: {event}").into());
                }
                let delta = if chat {
                    event["choices"][0]["delta"]["content"].as_str()
                } else if event["type"] == "response.output_text.delta" {
                    event["delta"].as_str()
                } else {
                    None
                };
                if let Some(delta) = delta.filter(|delta| !delta.is_empty()) {
                    first_delta.get_or_insert(elapsed);
                    last_delta = Some(elapsed);
                    delta_events += 1;
                    output_bytes += delta.len();
                }
                if !event["usage"].is_null() {
                    usage = event["usage"].clone();
                }
                if matches!(
                    event["type"].as_str(),
                    Some("response.completed" | "response.incomplete")
                ) {
                    usage = event["response"]["usage"].clone();
                    completed = true;
                }
            }
        }
        let total_s = start.elapsed().as_secs_f64();
        if !completed || first_delta.is_none() {
            return Err("Stream ended without a complete text response".into());
        }
        let metrics = client
            .get(format!("{base}/metrics"))
            .send()
            .await?
            .json::<Value>()
            .await?;
        println!(
            "{}",
            json!({"run":run,"endpoint":endpoint,"repetitions":repetitions,
            "headers_s":headers_s,"first_event_s":first_event,"first_text_s":first_delta,
            "last_text_s":last_delta,"total_s":total_s,"text_delta_events":delta_events,
            "output_bytes":output_bytes,"usage":usage,"server":metrics["latest"]})
        );
    }
    Ok(())
}
