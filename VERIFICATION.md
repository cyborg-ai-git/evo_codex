# Native provider verification

Verification was performed on macOS on 2026-09-25.

## Performance follow-up, 2026-09-26

The later [local model performance investigation](LOCAL_MODEL_PERFORMANCE.md) contains 19 completed, sequential requests against the installed 27B model, GPU/Metal observations, cold/warm timing, cache reuse and memory-limit checks, and analysis of the local response path. The [machine-readable measurements](benchmarks/mlx-latency-2026-09-26.json) and the Rust `mlx_latency_probe` example make the comparison reproducible. Temporary server settings were restored, and a final local response plus health/settings reads confirmed the original configuration and loaded 27B model.

## MLX downloads and hardware panel

The follow-up implementation adds disk discovery, five downloadable MLX models, resumable Rust downloads, an owned MLX server lifecycle, and a hardware sidebar. The checks below supersede the local-inference limitations recorded for the earlier provider work farther down this document.

- The final production `cargo build --release --locked -p codex-cli` passed in 23m 28s. The existing two unused-import warnings in `codex-cloud-tasks` remain. The updated release executable is installed in this checkout's `codex-rs/target/release/codex`.
- The updated `run_prod.sh` passed `--version`, `resume --help`, and `fork --help`. A final 140-column by 38-row PTY launch displayed the MLX catalog and hardware sidebar, verified the installed 2B checkpoint, accepted the next prompt without reopening a picker, and returned `READY` through the automatically started local runtime. This confirms the production input-guard fix as well as the launcher path.

- The broad follow-up run exercised 5,706 tests across TUI, provider, provider-info, and app-server model-list coverage: 5,696 passed initially. Seven failures were animation/timer snapshots that passed in isolation. A daemon warning snapshot exposed a narrower test viewport after enabling the hardware panel; the shared PTY test harness now disables the panel, which has its own snapshot coverage. That integration test also passed on rerun. The two existing numeric-locale failures remain.
- The isolated regression run passed 234 of 234 tests, including provider discovery, hardware parsing/rendering, model selection, download state, model-list RPCs, and the affected TUI integration checks.
- The final download-focused run passed 11 of 11 tests. It includes real HTTPS downloads of the small configuration file from each of the five pinned Hugging Face model revisions, with exact size and SHA-256 verification. Large model weights were not downloaded as part of this test.
- Download tests cover resumable partial files, range validation, integrity failures, existing files, and cancellation state. The UI regression test verifies that finishing preparation releases the settings input guard so the next prompt can be submitted.
- Four intended picker/download/hardware snapshots were reviewed and accepted. Unrelated snapshot differences were left unchanged.
- `sh -n run_prod.sh` passed. Scoped Clippy and Rust formatting were run after the tests, as required by `AGENTS.md`.

### Real local inference on this Mac

- A real terminal session displayed both existing checkpoints from `/Users/max/Documents/Model`: `Qwen3.5-2B-4bit` and `Qwen3.8-27B-Uncensored-MLX-4bit`. The other three catalog entries displayed download sizes and minimum RAM requirements.
- The Rust preparation flow verified the installed 2B model. Codex started the existing MLX-VLM runtime and received a streamed response through `/v1/responses`.
- A fresh task instructed the 2B model to invoke `exec_command`. The recorded tool call ran `printf 'MLX_TOOL_OK\\n'`, exited with code 0, returned `MLX_TOOL_OK`, and was followed by the model's final response. This verifies a real function-call/result cycle, not merely a model assertion that a command ran.
- The initial right panel displayed Apple M5 CPU/GPU information, RAM usage, and changing GPU utilization during inference. Temperature fields initially showed `N/A` because that implementation did not read AppleSMC sensors. The temperature follow-up below adds this missing reader.
- Normal CLI shutdown stopped the MLX server owned by that CLI; a subsequent connection to port 8080 failed as expected.
- The 27B checkpoint was discovered, but was not loaded or benchmarked. A successful 2B tool call does not guarantee that every local model follows instructions or supports every Codex tool reliably.
- Provider integration, downloads, and the sidebar are Rust. The existing MLX-VLM inference runtime uses Python/MLX externally; this change does not convert that engine to Rust. Linux/Windows builds and CPU/CUDA inference were not exercised.

## Earlier provider integration baseline

## Static checks

- The final `cargo build --release --locked -p codex-cli` passed. The updated executable is `codex-rs/target/release/codex`.
- The release executable returned successfully for `--version` (`codex-cli 0.0.0`) and `--help`. The sandbox prevented optional PATH alias creation; both commands still exited successfully.
- After fixing source-build startup to use `--no-daemon`, `run_prod.sh` was launched in a real PTY. The TUI loaded its model and working directory, and `/model` displayed OpenAI, DeepSeek API, Ollama, and LM Studio. The session exited normally without submitting an inference prompt. Explicit `--no-daemon`, `resume --help`, and `fork --help` also passed through the launcher.
- `cargo check --locked -p codex-model-provider -p codex-tui` passed.
- Scoped `just fix` / Clippy completed successfully for the six changed runtime/protocol crates and their tests.
- `just fmt` completed successfully. Stable rustfmt reports that the repository's nightly-only `imports_granularity` preference is ignored.

## Functional checks

- The selected provider, models-manager, TUI, app-server protocol, and model-list suites exercised 6,052 tests. Across the initial run and isolated reruns, 6,050 passed; the two remaining failures are listed below.
- All 24 focused provider/catalog/cursor/architecture tests passed, including provider-specific catalogs, local custom model IDs, managed provider restrictions, stale picker replies, task preservation, and model changes within a task.
- All 13 OpenAI picker animation and timing regression checks passed in isolation.
- A final run of 33 provider switching, model picker, collaboration catalog, and architecture regression tests passed after synchronizing the app and session catalogs on provider changes.
- The core DeepSeek integration test passed a simulated Responses streaming round trip with an actual shell tool execution and a subsequent assistant response. It verified separate DeepSeek authorization, no ChatGPT account header, and no OpenAI-hosted tool declarations.
- The protocol serialization check passed with the new optional `modelProvider` request field. Standard and experimental protocol schema fixtures were regenerated with the Rust fixture writer.
- The new provider and DeepSeek picker snapshots were reviewed and accepted. Unrelated snapshot changes were not accepted.

Some initial runs were affected by sandbox restrictions on localhost listeners, replacement of a test executable during a concurrent rebuild, terminal color settings, or timing under build load. The affected cases were rerun with local networking available and no concurrent rebuild of their test executable. Cursor tests passed with `NO_COLOR` unset, and timing-sensitive checks passed one at a time.

## Remaining locale-dependent test failures

These existing TUI tests expect English numeric grouping, while this Mac uses Italian number formatting:

- `chatwidget::tests::status_and_layout::rolling_rate_limit_snapshot_preserves_prior_individual_limit`
- `status::tests::status_snapshot_includes_enterprise_monthly_credit_limit`

The observed differences include `8,000` versus `8000` and `25,000` versus `25.000`. The number-formatting implementation and these snapshots were not changed. The selected suite is therefore not reported as a completely green single run.

## Live inference and platform limits

- No live DeepSeek API inference was performed. It requires a valid `DEEPSEEK_API_KEY`.
- Neither Ollama on port 11434 nor LM Studio on port 1234 was running. Local discovery was verified against a mock HTTP server, including malformed/oversized catalogs, errors, deduplication, and custom uncensored model names.
- A local server must support Responses streaming and function tools. Discovery alone cannot verify a model's tool-call quality or context capacity.
- CPU, NVIDIA CUDA, and Apple Metal execution are provided by the local inference server. No hardware inference benchmarks or device-specific execution tests were performed.
- Linux and Windows builds were not executed. The complete Rust workspace test suite was not executed; testing was scoped to the changed components and the DeepSeek core integration.

## macOS temperature follow-up (2026-09-26)

- Added a native Rust AppleSMC reader through IOKit. It discovers supported CPU/GPU temperature keys once, caches their metadata, and samples them on a blocking worker. The displayed temperature is the hottest valid sensor reading for each component. Only SMC read operations are implemented.
- A standalone executable built from the production reader sampled the user's Apple M5 without `sudo`: CPU 43.03–43.44 °C and GPU 42.28–42.70 °C over three readings. Initial discovery took approximately 42 ms; subsequent readings took 6.8–8.3 ms. These were ambient workload measurements, not a thermal stress benchmark.
- All seven scoped hardware tests passed through `just test -p codex-tui --lib -E 'test(hardware)'`. Coverage includes Apple Silicon float and Intel fixed-point decoding, rejection of invalid/inactive values, CPU/GPU sensor classification, missing-sensor rendering, temperature rendering, and narrow terminal behavior. The new temperature snapshot was reviewed and accepted.
- No Rust dependencies, model settings, fan controls, or inference code were changed. The existing Linux sensor path is preserved; Linux and Intel hardware were not exercised live.
