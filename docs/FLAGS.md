# CLI Flags

This engine currently uses runtime flags instead of a built-in `--help` command. Use the flags below after `cargo run --`.

## Editor

Default editor launch:

```powershell
cargo run --features editor
```

Enable benchmark tools inside the editor UI:

```powershell
cargo run --features editor -- --editor-benchmark-tools
```

`--editor-benchmark-tools`
- Shows `File > Benchmark > Load Benchmark Scene`
- Shows `File > Benchmark > Run CPU Benchmark`
- Makes `content/scenes/benchmark.scene.ron` visible in the asset browser

## Benchmark Runner

Run the benchmark directly:

```powershell
cargo run --release -- --benchmark
```

Useful benchmark flags:

`--benchmark`
- Starts the dedicated benchmark runner instead of the normal app/editor.

`--uncapped`
- Requests `Immediate` present mode for the benchmark so it is less likely to track display refresh pacing.

`--warmup <frames>`
- Number of warmup frames before sampling begins.

`--samples <frames>`
- Number of sampled frames written into the report.

`--seed <value>`
- Random seed used for deterministic benchmark content generation.

`--entities <count>`
- Requested benchmark entity count when the generated benchmark scene is used.
- If `content/scenes/benchmark.scene.ron` already exists, that saved scene takes precedence.

`--width <pixels>`
- Benchmark window width.

`--height <pixels>`
- Benchmark window height.

Example:

```powershell
cargo run --release -- --benchmark --uncapped --warmup=100 --samples=500 --seed=42 --width=1920 --height=1080
```

## Notes

- Benchmark UI/tools are hidden in normal editor runs unless `--editor-benchmark-tools` is present.
- Benchmark reports are written to `benchmarks/baseline_*.ron`.
