# Local model performance verification

Measurements were taken on 2026-09-26 on the user's Apple M5 Mac with 32 GiB of unified memory. The model was `/Users/max/Documents/Model/Qwen3.8-27B-Uncensored-MLX-4bit`, served by the existing MLX-VLM runtime on localhost. Other model generations were paused. No model tools were executed by the benchmark.

## What caused the reported wait

The actual Evo Codex server log recorded a 24,726-token input, 129.931 seconds of prefill, zero cached input tokens, and 76 generated tokens at 6.72 tokens/second. Total server request time was 149.233 seconds. Its metrics reported thinking disabled and zero reasoning tokens. The next turn queued 24,824 input tokens and began prefill again. The server health endpoint reported `apc_enabled: false`.

This is a large agent conversation, not just the short text typed into the composer. Codex sends agent instructions, tool definitions, workspace instructions, and accumulated conversation. The default local-model agent instructions alone are about 21 KB. In contrast, `qwen-tui` starts with an empty message list, sends Chat Completions requests without that agent setup, and disables thinking by default.

The screenshot's `Working` time is not a decode-speed measurement. It can include model loading, queued work, prefill, generation, and tools. Automatic thread-title generation can also use the same local model; earlier logs show title work competing with an agent request. That additional work was excluded from the controlled HTTP benchmarks.

## Controlled measurements

The Rust probe measures the first nonempty text delta, not the first HTTP header or SSE status event. It also records the server's tokenizer-based throughput; SSE chunks are not counted as tokens. Both endpoints use the same model, temperature 0, thinking disabled, and a request to count numbers. Runs are sequential. Most trials request 96 output tokens; the large-context trial requests 32.

| Case | Input tokens | First visible text | Server decode rate |
| --- | ---: | ---: | ---: |
| Chat Completions, first short request | 33 | 2.580 s | 7.27 tok/s |
| Chat Completions, two warm short requests | 33 | 0.544–0.554 s | 7.56–7.84 tok/s |
| Responses, three warm short requests | 33 | 0.574–0.622 s | 6.76–7.27 tok/s |
| Responses, two identical long requests, cache disabled | 8,433 | 39.009–39.071 s | 7.29–7.57 tok/s |
| Responses, two repeated long requests, populated cache | 8,433 | 0.516–0.594 s | 5.22–5.34 tok/s |
| Responses with a function declaration, initial cache fill | 8,695 | 45.158 s | 6.55 tok/s |
| Same function declaration and prompt, populated cache | 8,695 | 0.445 s | 6.18 tok/s |
| Follow-up turn with the same function declaration | 8,734 | 1.089 s | 6.12 tok/s |
| Short Responses control after restoring settings, two warm requests | 33 | 0.577–0.624 s | 7.16–7.66 tok/s |
| Large context with a function declaration, first run after reload | 24,445 | 198.832 s | 6.69 tok/s |
| Same large request repeated, cache enabled but no reusable entry | 24,445 | 137.615 s | 6.85 tok/s |

The cache test used `apc_enabled=true`, `apc_disk_enabled=false`, and `apc_memory_max_gb=2`. The repeated long prompt reused 8,432 tokens. The repeated tool-bearing prompt and its follow-up reused 8,694 tokens. This demonstrates reuse with an actual declared function and a growing conversation, not only exact text replay without tools.

The larger 24,445-token trial did **not** retain a usable cache entry with these limits. After its first run, the cache reported zero exact stores/hits and three memory skips, with a 2 GiB retention budget and approximately 3.30 GiB of prefill reserve. The second identical request started prefill from the beginning. The sub-second results above therefore must not be extrapolated to the user's approximately 25,000-token conversation. Increasing cache limits on this already memory-constrained machine was not tested or enabled by default.

All 19 measured requests completed. Machine-readable results are saved in [benchmarks/mlx-latency-2026-09-26.json](benchmarks/mlx-latency-2026-09-26.json). This is a small local experiment on a machine with other applications open, not a hardware-wide benchmark or a claim about every model. The existing `qwen-tui` request code was inspected; the comparison replays equivalent short requests through the same server rather than timing user interaction with its UI.

The first request after enabling the cache took 90.007 seconds to first text, including a model reload and first cache fill. It is not a warm-cache result. After restoring the original settings, the first short request took 35.434 seconds, while decode returned to 7.52 tok/s. Reloads and memory residency substantially affect cold measurements.

The cache improved repeated-prompt latency but did not improve decode throughput in this run. The operating system was also under memory pressure: swap allocation was already about 12.8 GiB before the cache experiment, and both swap-in and swap-out counters increased during the experiment and reloads. These measurements do not isolate cache cost from thermal state, memory residency, or other applications. Do not assume a speedup for every prompt or every context length.

## Metal verification

During generation, the Apple GPU driver reported 98% device utilization and 97% renderer utilization; the MLX process used approximately 13% CPU at that sample. MLX uses its default device stream in the installed generation implementation. These observations support GPU/Metal execution, not CPU-only inference. GPU counters are system-wide, not a per-process profiler. The implementation used for these benchmarks did not collect temperature or throttling status. A subsequent AppleSMC temperature reader fixes the missing macOS temperature fields; its later measurements cannot establish the thermal state during the earlier benchmarks.

The installed server also records `mx.get_peak_memory()`. On macOS this reads the [MLX Metal allocator](https://github.com/ml-explore/mlx/blob/main/mlx/backend/metal/allocator.cpp), providing process-side evidence of Metal memory allocation. This is a process high-water mark, not the current footprint or an isolated measurement for each request. The controlled HTTP runs do not involve Codex's TUI or hardware panel, so their approximately 7 tok/s decode rate cannot be attributed to UI rendering delays.

## Response behavior and permissions

The inspected local provider sends requests to MLX and does not use OpenAI authentication. The reported refusal is present in the rollout as an assistant message from the local-model task. No external OpenAI moderation request or local replacement-text filter was found on the inspected inference path.

The local model still receives Codex's coding-agent instructions, repository instructions, and prior conversation. Those differ from `qwen-tui` and can change model behavior. A checkpoint's name does not guarantee a particular answer, and this inspection does not identify the exact internal reason for an individual refusal.

The codebase also has handlers for server-supplied moderation/safety-buffering metadata. These handlers are not evidence that a local text classifier ran: the corresponding path requires the server to send those events. Tool approvals, sandbox restrictions, and optional approval review are separate controls on actions. They were not disabled or modified by this investigation.

## Historical-topic behavior checks

Five additional live checks on 2026-09-26 used the same 27B checkpoint. The Tiananmen 1989 question received substantive answers through Chat Completions, Responses, and the actual Evo Codex `exec` command. A Cultural Revolution question received substantive answers through Chat Completions and Evo Codex `exec`. All five completed with `finish_reason=stop`, without an explicit refusal. Both Evo tasks completed without tool calls. Raw prompts, responses, events, and server metrics are saved in [benchmarks/mlx-history-2026-09-26.json](benchmarks/mlx-history-2026-09-26.json).

These results show that those two historical topics were answered in the tested configuration. They do not establish the absence of all guardrails or guarantee other answers. The model's historical claims were not fact-checked and must not be treated as verified references. No weapon-construction prompt was submitted, and no safety controls or server settings were changed.

The new measurements also expose an important difference from the controlled latency benchmark: the direct historical requests explicitly disabled thinking, while the server reported `thinking_enabled=true` for the Evo requests under the existing configuration. Tiananmen used 7,677 input tokens and 838 generated tokens in 160.416 seconds; the Cultural Revolution used 7,669 input tokens and 1,879 generated tokens in 290.499 seconds. Both reported zero reasoning tokens despite thinking being enabled, so that counter alone cannot establish that reasoning is disabled. These are behavioral comparisons, not matched latency trials. The earlier request's reported `thinking_enabled=false` remains a separate observation about that request.

## Reproduce the harmless benchmark

The scoped `codex-api` SSE and safety-metadata regression run passed 43 of 43 tests, using `just test`/nextest. It includes preservation of output events while server-supplied safety-buffering notifications are handled. This verifies transport behavior; it is not a claim that every model must answer every request. The probe itself was built and exercised against the real server in all 19 recorded trials, then checked with scoped Clippy and formatting.

Build the Rust-only probe from `codex-rs`:

```sh
cargo build --locked -p codex-model-provider --example mlx_latency_probe
target/debug/examples/mlx_latency_probe \
  http://127.0.0.1:8080 \
  /Users/max/Documents/Model/Qwen3.8-27B-Uncensored-MLX-4bit \
  responses 400 3
```

Use `chat` instead of `responses` to compare protocols, or `0` instead of `400` for the short prompt. The final optional argument is a JSON request file for controlled replay. The probe refuses to start a run if the server reports an active request. It does not change server settings. JSON output includes separate header, first-event, first-text, total-time, usage, and server metrics.

Prefix caching is a feature of the external MLX server. See the upstream [MLX-VLM cache documentation](https://github.com/Blaizzy/mlx-vlm#automatic-prefix-caching-apc). A first uncached request still needs prefill, and cache entries can be evicted under memory pressure. Starting a new Codex task with `/new` avoids resending an unrelated conversation; it does not remove the agent's instructions or tools.

For an optional short-context experiment, close the CLI that owns the MLX server, then start it with:

```sh
APC_ENABLED=1 APC_DISK_ENABLED=0 APC_MEMORY_MAX_GB=2 ./run_prod.sh
```

These variables affect a newly started MLX child. They do not reconfigure a server that is already running. This bounded setting was effective around 8,700 input tokens in the tests and ineffective around 24,400; it is not a universal optimization. The benchmark temporarily changed the live server through its settings API and restored the saved original values afterward. No inference defaults or safety/approval settings were changed in the repository.
