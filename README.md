# Concurrent Ollama inference

Send real `/api/generate` requests from independent concurrent clients. Each client sends its requests sequentially and reuses its own HTTP connection pool.

```sh
cargo run --release -- --sessions 4 --requests 3 --compare
```

The default URL is `http://localhost:11434`. When exactly one model is installed it is selected automatically. Otherwise pass `--model NAME` (see `ollama list`). No model is downloaded automatically.

```sh
cargo run --release -- --url http://localhost:11434 --model MODEL_NAME \
  --sessions 8 --requests 5 --tokens 128 --timeout 120000 \
  --prompt 'Explain how a CPU works in a short paragraph.' --compare
```

- `--compare` runs a one-client baseline before the concurrent round, with the same requests per client. The ratio compares measured aggregate token throughput, not a sum of latencies. The concurrent round does more total work.
- A one-token warmup is excluded from measurements. Use `--no-warmup` to include model loading. Prompt caching and round order can affect comparisons. Repeat runs and use representative prompts for meaningful results.
- Per-request latency includes HTTP, server queueing, prompt processing and full generation. `generation` tokens/s comes from Ollama's `eval_count / eval_duration`. `load` reports Ollama's model loading time.
- Aggregate tokens/s is total output tokens from successful completed requests divided by measured round wall time. Failed or cancelled partial outputs are excluded. Mean, p50 and p95 describe successful requests only. Small samples produce unstable percentiles.
- Responses use `stream: false`. This measures full response latency, not time to first token. Model thinking behavior follows its default and may consume the output token budget.
- `--timeout MS` bounds the complete request, including response reading. `--duration S` bounds each measured round, excludes discovery and warmup, and cancels outstanding work. Cancelled counts include queued requests not yet started. Ctrl+C cancels the active round. Client cancellation does not guarantee instantaneous termination of work on the server.
- Failures retain their cause, count in the summary and produce a nonzero exit code. Bodies are limited to 16 MiB. CLI errors exit with code 2.
- More connected clients do not guarantee more simultaneous GPU inference. Ollama scheduling, available memory, context size and server parallelism determine actual execution. This tool does not change your server configuration.

API contract: https://docs.ollama.com/api/generate

## Verification

```sh
cargo test
cargo clippy --all-targets -- -D warnings
```

The integration suite runs the real binary against a local HTTP server and checks concurrent requests, JSON payloads, HTTP failures, incomplete or malformed responses, request timeout, round deadline and invalid arguments. It requires permission to bind loopback sockets. It does not simulate model inference performance.

## Code layout

- `src/main.rs`: entry point and process exit codes.
- `src/config.rs`: CLI parsing and validation.
- `src/ollama.rs`: model discovery, HTTP requests and response validation.
- `src/benchmark.rs`: warmup, concurrent clients, cancellation and comparison.
- `src/report.rs`: statistics and summary formatting.
- `tests/integration.rs`: end-to-end tests with a local HTTP server, using Rust only.
