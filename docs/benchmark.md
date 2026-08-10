# Release smoke benchmark

The v1 performance target is a warm median invocation below 50 ms. This is a smoke check, not a gating benchmark: filesystem, SQLite, process-launch, power, and virtualization differences can materially affect it.

Build the release binary, initialize an isolated database, and time at least 101 warm read-only invocations of `t ls` using a monotonic, high-resolution clock. Report the median wall-clock duration and the host/toolchain context.

```sh
cargo build --release
benchmark_data=$(mktemp -d /tmp/t-benchmark.XXXXXX)
XDG_DATA_HOME="$benchmark_data" target/release/t add 1 benchmark >/dev/null
# Time 101 invocations of: XDG_DATA_HOME="$benchmark_data" target/release/t ls
```

Latest local result (2026-08-10): **10.923 ms median**, with 9.746 ms minimum, 11.433 ms p95, and 11.922 ms maximum across 101 measured invocations after five warmups. This passes the `<50 ms` target.

The measurement used the release profile on x86_64 macOS 15.7.7 with `rustc 1.95.0`. Child stdout was redirected to `/dev/null`; timing included process launch, database open, scheduling, rendering, and process exit.
