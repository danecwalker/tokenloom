//! `tokenloom update` — self-updater.
//!
//! Mirrors `install.sh`: resolves the latest GitHub Release, downloads the
//! platform archive (`tokenloom-<tag>-<target>.<ext>`), verifies the
//! published sha256 checksum, unpacks with the system `tar`, and replaces
//! the running binary (atomic rename on POSIX; old-swap dance on Windows,
//! where a running executable cannot be renamed over).

use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use tokenloom_core::Config;
use tokenloom_fetch::FetchClient;

const REPO: &str = "danecwalker/tokenloom";
/// Release archives are a few MiB; refuse to buffer anything larger.
const DOWNLOAD_CAP: u64 = 256 * 1024 * 1024;

pub async fn run(config: &Config, check_only: bool, pin: Option<String>, force: bool) -> i32 {
    match update(config, check_only, pin, force).await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("tokenloom: update failed: {e}");
            1
        }
    }
}

async fn update(
    config: &Config,
    check_only: bool,
    pin: Option<String>,
    force: bool,
) -> Result<(), String> {
    let current = env!("CARGO_PKG_VERSION");
    let client = FetchClient::new(&config.http).map_err(|e| e.to_string())?;

    // Resolve the release to move to: pinned tag or latest.
    let api = match pin.as_deref() {
        Some(v) => format!(
            "https://api.github.com/repos/{REPO}/releases/tags/v{}",
            v.trim().trim_start_matches('v')
        ),
        None => format!("https://api.github.com/repos/{REPO}/releases/latest"),
    };
    let (tag, assets) = fetch_release(&client, &api).await?;

    let target_v = parse_version(&tag).ok_or_else(|| format!("unparseable release tag '{tag}'"))?;
    let current_v = parse_version(current).ok_or("unparseable local version")?;
    let platform_asset = platform()
        .and_then(|(target, ext)| find_asset(&assets, target, ext).map(|u| (target, ext, u)));

    if check_only {
        let status = match target_v.cmp(&current_v) {
            Ordering::Greater => match &platform_asset {
                Some((_, _, url)) => {
                    let name = url.rsplit('/').next().unwrap_or(url);
                    format!("update available: {tag} ({name})")
                }
                None => format!("update available: {tag} (no prebuilt asset for this platform)"),
            },
            Ordering::Less => format!("newer than latest release {tag}"),
            Ordering::Equal => "up to date".to_string(),
        };
        println!("tokenloom v{current} — {status}");
        return Ok(());
    }

    let Some((_target, ext, asset_url)) = platform_asset else {
        return Err(format!(
            "no prebuilt release asset for {} / {} — update with `cargo install --path crates/tokenloom` or install.sh instead",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
    };
    let asset_name = asset_url
        .rsplit('/')
        .next()
        .unwrap_or(&asset_url)
        .to_string();

    // An explicit `--to` may downgrade; the implicit latest never goes back.
    let allow = pin.is_some() || force || target_v > current_v;
    if !allow {
        if target_v == current_v {
            println!("tokenloom v{current} — already up to date ({tag})");
        } else {
            println!("tokenloom v{current} — newer than latest release {tag}; nothing to do");
        }
        return Ok(());
    }

    eprintln!("tokenloom: downloading {asset_name}…");
    let archive = download_capped(client.raw(), &asset_url, DOWNLOAD_CAP).await?;
    eprintln!("tokenloom: downloaded {} bytes", archive.len());

    match download_capped(client.raw(), &format!("{asset_url}.sha256"), 64 * 1024).await {
        Ok(sum) if verify_checksum(&archive, &String::from_utf8_lossy(&sum)) => {
            eprintln!("tokenloom: checksum OK");
        }
        Ok(_) => {
            return Err(
                "checksum mismatch — the archive does not match the published sha256".into(),
            )
        }
        Err(_) => eprintln!("tokenloom: (no checksum published; skipping verification)"),
    }

    let tmp = std::env::temp_dir().join(format!("tokenloom-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("cannot create temp dir: {e}"))?;
    let archive_path = tmp.join(&asset_name);
    let result = (|| -> Result<(PathBuf, Option<String>), String> {
        std::fs::write(&archive_path, &archive)
            .map_err(|e| format!("cannot write archive: {e}"))?;
        extract(&archive_path, &tmp, ext)?;
        let binary =
            find_binary(&tmp).ok_or("archive layout unexpected — no tokenloom binary found")?;
        // Ask the new binary what version it actually is before replacing
        // anything: catches release packaging drift (tag ≠ manifest).
        let reported = binary_version(&binary);
        if let Some(reported) = &reported {
            let reported_v = reported.split_whitespace().last().and_then(parse_version);
            if reported_v != Some(target_v) {
                eprintln!(
                    "tokenloom: note — the {tag} binary self-reports '{reported}'; \
                     the release tag and the packaged version differ"
                );
            }
        }
        let installed = replace_binary(&binary)?;
        Ok((installed, reported))
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    let (installed, reported) = result?;

    match reported {
        Some(reported) => println!("updated tokenloom v{current} → {tag} ({reported})"),
        None => println!("updated tokenloom v{current} → {tag}"),
    }
    println!("installed at {}", installed.display());
    eprintln!("tokenloom: running sessions keep the old binary until restarted");
    Ok(())
}

/// GET a GitHub Releases API URL and return `(tag_name, assets)`. Routed
/// through [`FetchClient::get_capped`], so the SSRF guard, timeouts and byte
/// cap all apply (the API now answers `application/json`, which the reader
/// pipeline accepts).
async fn fetch_release(
    client: &FetchClient,
    url: &str,
) -> Result<(String, serde_json::Value), String> {
    let raw = client
        .get_capped(url)
        .await
        .map_err(|e| format!("cannot reach GitHub Releases: {e}"))?;
    if raw.status != 200 {
        return Err(format!("GitHub Releases returned HTTP {}", raw.status));
    }
    let v: serde_json::Value = serde_json::from_slice(&raw.body)
        .map_err(|e| format!("unexpected GitHub API response: {e}"))?;
    let tag = v
        .get("tag_name")
        .and_then(|t| t.as_str())
        .ok_or("GitHub API response missing tag_name")?
        .to_string();
    let assets = v
        .get("assets")
        .cloned()
        .ok_or("GitHub API response missing assets")?;
    Ok((tag, assets))
}

/// Stream a download with a hard byte cap (same decompression-bomb posture
/// as the reader pipeline). Uses the raw client: release assets are
/// `application/octet-stream`, which the reader pipeline rightly rejects.
async fn download_capped(http: &reqwest::Client, url: &str, cap: u64) -> Result<Vec<u8>, String> {
    let mut resp = http
        .get(url)
        .send()
        .await
        .map_err(|e| format!("download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download failed: HTTP {}", resp.status()));
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("download interrupted: {e}"))?
    {
        if body.len() as u64 + chunk.len() as u64 > cap {
            return Err(format!(
                "asset exceeds the {} MiB update cap",
                cap / (1024 * 1024)
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// `(release target triple, archive extension)` for this machine, mirroring
/// install.sh and the release workflow matrix.
fn platform() -> Option<(&'static str, &'static str)> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some(("x86_64-unknown-linux-musl", "tar.gz")),
        ("linux", "aarch64") => Some(("aarch64-unknown-linux-musl", "tar.gz")),
        ("macos", "aarch64") => Some(("aarch64-apple-darwin", "tar.gz")),
        ("macos", "x86_64") => Some(("x86_64-apple-darwin", "tar.gz")),
        ("windows", "x86_64") => Some(("x86_64-pc-windows-msvc", "zip")),
        _ => None,
    }
}

/// Find the release asset for `target` in the release's asset list.
fn find_asset(assets: &serde_json::Value, target: &str, ext: &str) -> Option<String> {
    let suffix = format!("-{target}.{ext}");
    assets.as_array()?.iter().find_map(|a| {
        let name = a.get("name")?.as_str()?;
        if !name.ends_with(&suffix) {
            return None;
        }
        a.get("browser_download_url")?.as_str().map(str::to_string)
    })
}

/// Parse `v0.1.7` / `0.1.7` / `0.2.0-rc.1` into (major, minor, patch).
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    Some((major, minor, patch))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(2 * digest.len());
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Check the archive against a sha256sum-format checksum file
/// (`<hex>  <filename>`), case-insensitive on the hex.
fn verify_checksum(archive: &[u8], checksum_file: &str) -> bool {
    let expected = checksum_file.split_whitespace().next().unwrap_or("");
    !expected.is_empty() && sha256_hex(archive) == expected.to_ascii_lowercase()
}

/// Unpack the archive with the system `tar` (bsdtar on macOS and Windows 10+
/// handles both formats; no extra crates or shell scripting needed).
fn extract(archive: &Path, dest: &Path, ext: &str) -> Result<(), String> {
    let mut cmd = std::process::Command::new("tar");
    if ext == "zip" {
        cmd.arg("-xf");
    } else {
        cmd.arg("-xzf");
    }
    let status = cmd
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .status()
        .map_err(|e| format!("could not run system 'tar' to unpack the release: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("tar exited with {status}"))
    }
}

/// Locate the extracted `tokenloom` binary anywhere inside the archive tree.
fn find_binary(dir: &Path) -> Option<PathBuf> {
    let wanted = if cfg!(windows) {
        "tokenloom.exe"
    } else {
        "tokenloom"
    };
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|n| n == wanted) {
                return Some(path);
            }
        }
    }
    None
}

/// Ask an extracted binary for its self-reported version (`--version`).
/// Best-effort: `None` when the binary won't run or prints nothing.
fn binary_version(binary: &Path) -> Option<String> {
    let out = std::process::Command::new(binary)
        .arg("--version")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Install `new_binary` over the running executable. POSIX allows renaming
/// over a running binary (atomic swap); Windows does not, so the running
/// image is moved aside first and rolled back if the swap fails. Returns the
/// install path.
fn replace_binary(new_binary: &Path) -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("cannot locate current executable: {e}"))?
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_exe().expect("current_exe existed above"));
    let name = exe
        .file_name()
        .ok_or("current executable has no file name")?
        .to_os_string();
    let staging = exe.with_file_name(format!(".{}.new", name.to_string_lossy()));
    std::fs::copy(new_binary, &staging).map_err(|e| format!("cannot stage new binary: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("cannot set permissions: {e}"))?;
        std::fs::rename(&staging, &exe).map_err(|e| format!("cannot replace binary: {e}"))?;
    }

    #[cfg(windows)]
    {
        let old = exe.with_file_name(format!(".{}.old", name.to_string_lossy()));
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&exe, &old)
            .map_err(|e| format!("cannot move the running binary aside: {e}"))?;
        if let Err(e) = std::fs::rename(&staging, &exe) {
            let _ = std::fs::rename(&old, &exe);
            return Err(format!("cannot replace the running binary: {e}"));
        }
        // Still locked by the running process; cleaned up by the next update.
        let _ = std::fs::remove_file(&old);
    }

    Ok(exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("v0.1.7"), Some((0, 1, 7)));
        assert_eq!(parse_version("0.1.7"), Some((0, 1, 7)));
        assert_eq!(parse_version("v0.1"), Some((0, 1, 0)));
        assert_eq!(parse_version("1.2.3-rc.1"), Some((1, 2, 3)));
        assert_eq!(parse_version("v2.0.0+build.5"), Some((2, 0, 0)));
        assert_eq!(parse_version("junk"), None);
    }

    #[test]
    fn platform_matches_the_host() {
        // Whatever the CI matrix builds for, the running host must map.
        let (target, ext) = platform().expect("host platform must be supported");
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        if os == "linux" || os == "macos" {
            assert_eq!(ext, "tar.gz");
        } else {
            assert_eq!(ext, "zip");
        }
        assert!(target.contains(arch.split('_').next().unwrap()));
    }

    #[test]
    fn finds_the_platform_asset() {
        let assets = serde_json::json!([
            {"name": "tokenloom-v0.1.7-x86_64-unknown-linux-musl.tar.gz",
             "browser_download_url": "https://github.com/x/tokenloom/releases/download/v0.1.7/tokenloom-v0.1.7-x86_64-unknown-linux-musl.tar.gz"},
            {"name": "tokenloom-v0.1.7-aarch64-apple-darwin.tar.gz",
             "browser_download_url": "https://github.com/x/tokenloom/releases/download/v0.1.7/tokenloom-v0.1.7-aarch64-apple-darwin.tar.gz"},
            {"name": "tokenloom-v0.1.7-x86_64-pc-windows-msvc.zip",
             "browser_download_url": "https://github.com/x/tokenloom/releases/download/v0.1.7/tokenloom-v0.1.7-x86_64-pc-windows-msvc.zip"}
        ]);
        let url = find_asset(&assets, "aarch64-apple-darwin", "tar.gz").unwrap();
        assert!(url.ends_with("tokenloom-v0.1.7-aarch64-apple-darwin.tar.gz"));
        assert_eq!(find_asset(&assets, "riscv64-unknown-linux", "tar.gz"), None);
        assert_eq!(
            find_asset(&assets, "x86_64-pc-windows-msvc", "tar.gz"),
            None
        );
        // Mismatched extension must not match a tar.gz asset.
        assert_eq!(find_asset(&assets, "aarch64-apple-darwin", "zip"), None);
    }

    #[test]
    fn verifies_sha256_checksums() {
        // sha256("hello world")
        let archive = b"hello world";
        let sum = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        assert!(verify_checksum(
            archive,
            &format!("{sum}  hello-world.tar.gz\n")
        ));
        assert!(verify_checksum(archive, &format!("{sum}  file"))); // sha256sum format
        assert!(verify_checksum(archive, sum)); // bare hex also accepted
        assert!(!verify_checksum(archive, "deadbeef"));
        assert!(!verify_checksum(archive, ""));
    }

    #[test]
    fn sha256_is_stable() {
        assert_eq!(
            sha256_hex(b"hello world"),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }
}
