use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::SystemTime;

use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use serde_json::Value;

const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";
const BAR_WIDTH: usize = 8;

fn main() {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let json: Value = serde_json::from_str(&input).unwrap_or(Value::Null);

    let mut out = String::new();

    let cwd = json
        .pointer("/workspace/current_dir")
        .and_then(Value::as_str)
        .unwrap_or("");
    out.push_str(&shorten_path(&tilde(cwd)));

    if let Some(branch) = git_branch(cwd) {
        out.push_str(&format!(" ({branch})"));
        if let Some(ci) = ci_status(cwd, &branch) {
            out.push_str(&format!(" | {ci}"));
        }
    }

    if let Some(model) = json
        .pointer("/model/display_name")
        .and_then(Value::as_str)
        .or_else(|| json.pointer("/model/id").and_then(Value::as_str))
    {
        match json.pointer("/effort/level").and_then(Value::as_str) {
            Some(effort) => out.push_str(&format!(" | [{effort}] {model}")),
            None => out.push_str(&format!(" | {model}")),
        }
    }

    if let Some(total) = json
        .pointer("/context_window/context_window_size")
        .and_then(Value::as_f64)
    {
        let used = json
            .pointer("/context_window/total_input_tokens")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            + json
                .pointer("/context_window/total_output_tokens")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
        out.push_str(&format!(" | {}/{}", humanize(used), humanize(total)));
        if let Some(pct) = json
            .pointer("/context_window/used_percentage")
            .and_then(Value::as_f64)
        {
            out.push_str(&format!(" ({}%)", fmt_num(pct)));
        }
    }

    // Claude.ai subscription usage: 5h session / 7d weekly limits,
    // each as a colored bar with percentage and reset time
    let mut rates: Vec<String> = Vec::new();
    for (label, key) in [("5h", "five_hour"), ("7d", "seven_day")] {
        let Some(pct) = json
            .pointer(&format!("/rate_limits/{key}/used_percentage"))
            .and_then(Value::as_f64)
        else {
            continue;
        };
        let mut s = format!("{label}:{:.0}%{}", pct, bar(pct));
        if let Some(reset) = json
            .pointer(&format!("/rate_limits/{key}/resets_at"))
            .and_then(format_reset)
        {
            s.push_str(&format!(" →{reset}"));
        }
        rates.push(s);
    }
    if !rates.is_empty() {
        out.push_str(&format!(" | {}", rates.join("  ")));
    }

    if let Some(acc) = account(&json) {
        out.push_str(&format!(" | [{acc}]"));
    }

    print!("{out}");
}

/// Replace a leading $HOME with `~`
fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => match path.strip_prefix(&home) {
            Some(rest) => format!("~{rest}"),
            None => path.to_string(),
        },
        _ => path.to_string(),
    }
}

/// Shorten long paths to "first/two/.../last" keeping the head and tail segments
fn shorten_path(path: &str) -> String {
    const MAX_LEN: usize = 40;
    if path.chars().count() <= MAX_LEN {
        return path.to_string();
    }
    let parts: Vec<&str> = path.split('/').collect();
    let n = parts.len();
    if n <= 4 {
        return path.to_string();
    }
    let first = if parts[0].is_empty() {
        format!("/{}/{}", parts[1], parts[2])
    } else {
        format!("{}/{}", parts[0], parts[1])
    };
    format!("{}/.../{}", first, parts[n - 1])
}

fn git_branch(cwd: &str) -> Option<String> {
    if cwd.is_empty() {
        return None;
    }
    let out = Command::new("git")
        .args(["-C", cwd, "--no-optional-locks", "branch", "--show-current"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!branch.is_empty()).then_some(branch)
}

/// CI + review status of the branch's PR, e.g. "CI:✔ Rv:👀".
/// Data comes from `gh pr checks` / `gh pr view`, cached per (cwd, branch) for
/// CI_TTL_SECS and refreshed by a detached background process so the
/// statusline itself never waits on the network.
fn ci_status(cwd: &str, branch: &str) -> Option<String> {
    const CI_TTL_SECS: u64 = 60;
    if cwd.is_empty() || matches!(branch, "main" | "master") {
        return None;
    }
    let home = std::env::var("HOME").ok()?;
    let cache_dir = Path::new(&home).join(".cache/claude-statusline");
    let key = format!("ci-{:016x}", fnv1a(&format!("{cwd}|{branch}")));
    let cache = cache_dir.join(format!("{key}.json"));
    if !file_age_secs(&cache).is_some_and(|age| age < CI_TTL_SECS) {
        let _ = std::fs::create_dir_all(&cache_dir);
        let lock = cache_dir.join(format!("{key}.lock"));
        // A recent lock means a refresh is already in flight (or gh is failing);
        // don't pile up subprocesses
        if !file_age_secs(&lock).is_some_and(|age| age < 2 * CI_TTL_SECS) {
            let _ = std::fs::write(&lock, b"");
            let tmp = cache_dir.join(format!("{key}.tmp"));
            let script = format!(
                "cd {cwd:?} || exit 0; \
                 c=$(gh pr checks --json name,bucket 2>/dev/null); \
                 r=$(gh pr view --json reviewDecision --jq .reviewDecision 2>/dev/null); \
                 printf '{{\"checks\":%s,\"reviewDecision\":\"%s\"}}' \"${{c:-[]}}\" \"$r\" > {tmp:?} \
                 && mv {tmp:?} {cache:?}; rm -f {lock:?}"
            );
            let _ = Command::new("sh")
                .args(["-c", &script])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
    }
    let data: Value = serde_json::from_str(&std::fs::read_to_string(&cache).ok()?).ok()?;
    render_ci(&data)
}

fn render_ci(data: &Value) -> Option<String> {
    // Checks tied to review approval (they stay pending until the PR is
    // approved) are excluded from the CI verdict so "only review left" shows
    // as a green CI with Rv:👀 instead of a forever-yellow CI
    let ignore: Vec<String> = std::env::var("STATUSLINE_CI_IGNORE")
        .unwrap_or_else(|_| "validate-review".to_string())
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let (mut pass, mut fail, mut pending) = (0, 0, 0);
    for check in data.pointer("/checks").and_then(Value::as_array)? {
        let name = check
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        if ignore.iter().any(|pat| name.contains(pat)) {
            continue;
        }
        match check.get("bucket").and_then(Value::as_str).unwrap_or("") {
            "pass" => pass += 1,
            "fail" | "cancel" => fail += 1,
            "pending" => pending += 1,
            _ => {} // "skipping"
        }
    }
    let review = data
        .pointer("/reviewDecision")
        .and_then(Value::as_str)
        .unwrap_or("");
    if pass + fail + pending == 0 && review.is_empty() {
        return None;
    }
    let mut s = String::from("CI:");
    if fail > 0 {
        s.push_str(&format!("{RED}✘{fail}{RESET}"));
    } else if pending > 0 {
        s.push_str(&format!("{YELLOW}●{pending}{RESET}"));
    } else {
        s.push_str(&format!("{GREEN}✔{RESET}"));
    }
    match review {
        "REVIEW_REQUIRED" => s.push_str(&format!(" Rv:{YELLOW}👀{RESET}")),
        "CHANGES_REQUESTED" => s.push_str(&format!(" Rv:{RED}✘{RESET}")),
        "APPROVED" => s.push_str(&format!(" Rv:{GREEN}✔{RESET}")),
        _ => {}
    }
    Some(s)
}

fn file_age_secs(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    SystemTime::now()
        .duration_since(modified)
        .ok()
        .map(|d| d.as_secs())
}

fn fnv1a(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Detect account type via `claude auth status --json`, cached per session_id
/// to avoid the ~200ms subprocess cost on every status refresh
fn account(json: &Value) -> Option<String> {
    let sid = json.pointer("/session_id").and_then(Value::as_str)?;
    let home = std::env::var("HOME").ok()?;
    let cache_dir = Path::new(&home).join(".cache/claude-statusline");
    let cache = cache_dir.join(format!("{sid}.auth.json"));
    if !cache.exists() {
        let _ = std::fs::create_dir_all(&cache_dir);
        if let Ok(out) = Command::new("claude").args(["auth", "status", "--json"]).output() {
            let _ = std::fs::write(&cache, &out.stdout);
        }
    }
    let data = std::fs::read_to_string(&cache).ok()?;
    if data.is_empty() {
        return None;
    }
    let auth: Value = serde_json::from_str(&data).ok()?;
    let sub = auth
        .get("subscriptionType")
        .and_then(Value::as_str)
        .unwrap_or("");
    let org = auth.get("orgName").and_then(Value::as_str).unwrap_or("");
    Some(match sub {
        "max" => "Max".to_string(),
        "pro" => "Pro".to_string(),
        // Prefer org name (e.g. "フリー株式会社") for visual distinction
        "enterprise" if !org.is_empty() => org.to_string(),
        "enterprise" => "Enterprise".to_string(),
        "" => "unknown".to_string(),
        other => other.to_string(),
    })
}

/// Colored usage bar, e.g. [███░░░░░] — green <70%, yellow <90%, red >=90%
fn bar(pct: f64) -> String {
    let filled = (((pct * BAR_WIDTH as f64 / 100.0) + 0.5) as usize).min(BAR_WIDTH);
    let color = if pct >= 90.0 {
        RED
    } else if pct >= 70.0 {
        YELLOW
    } else {
        GREEN
    };
    format!(
        "{color}[{}{}]{RESET}",
        "█".repeat(filled),
        "░".repeat(BAR_WIDTH - filled)
    )
}

/// Format resets_at (unix epoch number/string or ISO 8601) as local "Mon 14:32"
fn format_reset(v: &Value) -> Option<String> {
    let local: DateTime<Local> = match v {
        Value::Number(n) => Local.timestamp_opt(n.as_f64()? as i64, 0).single()?,
        Value::String(s) if s.chars().all(|c| c.is_ascii_digit() || c == '.') => {
            let epoch = s.split('.').next()?.parse::<i64>().ok()?;
            Local.timestamp_opt(epoch, 0).single()?
        }
        Value::String(s) => match DateTime::parse_from_rfc3339(s) {
            Ok(dt) => dt.with_timezone(&Local),
            // No offset in the string: assume UTC
            Err(_) => {
                let naive =
                    NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S")
                        .ok()?;
                Utc.from_utc_datetime(&naive).with_timezone(&Local)
            }
        },
        _ => return None,
    };
    Some(local.format("%a %H:%M").to_string())
}

/// 1234 -> "1.2K", 50000 -> "50K", 1500000 -> "1.5M"
fn humanize(n: f64) -> String {
    if n >= 1_000_000.0 {
        fmt_unit(n / 1_000_000.0, "M")
    } else if n >= 1000.0 {
        fmt_unit(n / 1000.0, "K")
    } else {
        format!("{}", n as i64)
    }
}

fn fmt_unit(v: f64, unit: &str) -> String {
    if v.fract() == 0.0 {
        format!("{}{unit}", v as i64)
    } else {
        format!("{v:.1}{unit}")
    }
}

/// Print a float without trailing ".0" (25.0 -> "25", 25.5 -> "25.5")
fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}
