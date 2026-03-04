use super::BenchmarkResults;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn write_report(results: &BenchmarkResults) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let benchmark_dir = PathBuf::from("benchmarks");
    fs::create_dir_all(&benchmark_dir)?;

    let filename = format!("baseline_{}.ron", timestamp_string());
    let path = benchmark_dir.join(filename);

    let pretty = ron::ser::PrettyConfig::new()
        .depth_limit(4)
        .separate_tuple_members(true)
        .enumerate_arrays(true);
    let report_text = ron::ser::to_string_pretty(results, pretty)?;
    fs::write(&path, report_text)?;

    Ok(path)
}

pub fn print_summary(path: &Path, results: &BenchmarkResults) {
    println!("Benchmark complete");
    println!("Report: {}", path.display());
    println!("Present mode: {}", results.metadata.present_mode);
    println!(
        "Frames: {} | Avg {:.2} ms | P99 {:.2} ms | StdDev {:.2} ms",
        results.frame_times_ms.len(),
        results.avg_frame_ms,
        results.p99_frame_ms,
        results.stddev_frame_ms,
    );
    println!(
        "Render: {} draws, {} tris, {} material changes, {} visible",
        results.render_counters.draw_calls,
        results.render_counters.triangles,
        results.render_counters.material_changes,
        results.render_counters.visible_entities,
    );
}

fn timestamp_string() -> String {
    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Get-Date -Format yyyy-MM-dd_HHmmss",
            ])
            .output()
        {
            if output.status.success() {
                if let Ok(timestamp) = String::from_utf8(output.stdout) {
                    let timestamp = timestamp.trim();
                    if !timestamp.is_empty() {
                        return timestamp.to_string();
                    }
                }
            }
        }
    }

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}
