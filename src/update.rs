use std::io::Read;
use std::sync::mpsc::{Receiver, channel};

const REPO_OWNER: &str = "Valou-31";
const REPO_NAME: &str = "RustyComicReader";
const USER_AGENT: &str = "RustyComicReader-updater";

/// What a newer release looked like when `spawn_check` found one.
#[derive(Clone, Debug)]
pub struct UpdateInfo {
    pub version: String,
    pub asset_url: String,
}

pub enum CheckEvent {
    Available(UpdateInfo),
    UpToDate,
    Failed(String),
}

pub enum ApplyEvent {
    Done,
    Failed(String),
}

#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// The release asset name for the platform this binary is running on — must
/// match exactly what `.github/workflows/release.yml` uploads.
fn asset_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "comic-reader-macos-arm64.zip"
    } else if cfg!(target_os = "windows") {
        "comic-reader-windows-x86_64.exe"
    } else {
        "comic-reader-linux-x86_64.tar.gz"
    }
}

/// Parses a version string ("0.1.4", or a release tag "v0.1.4") into a
/// comparable `(major, minor, patch)` tuple. An unparseable segment becomes
/// `0` — only reachable for a malformed release tag, since `CARGO_PKG_VERSION`
/// is always well-formed.
fn parse_version(raw: &str) -> (u32, u32, u32) {
    let raw = raw.strip_prefix('v').unwrap_or(raw);
    let mut parts = raw.split('.').map(|p| p.parse().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

/// Checks GitHub's latest release against the running version, on a
/// background thread — network access shouldn't block startup, and a
/// failure (offline, rate-limited, no matching asset) is meant to be logged
/// and ignored rather than bothering the user.
pub fn spawn_check() -> Receiver<CheckEvent> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(check());
    });
    rx
}

fn check() -> CheckEvent {
    let url = format!("https://api.github.com/repos/{REPO_OWNER}/{REPO_NAME}/releases/latest");
    let mut response = match ureq::get(&url).header("User-Agent", USER_AGENT).call() {
        Ok(response) => response,
        Err(err) => return CheckEvent::Failed(err.to_string()),
    };
    let body = match response.body_mut().read_to_string() {
        Ok(body) => body,
        Err(err) => return CheckEvent::Failed(err.to_string()),
    };
    let release: Release = match serde_json::from_str(&body) {
        Ok(release) => release,
        Err(err) => return CheckEvent::Failed(err.to_string()),
    };

    if parse_version(&release.tag_name) <= parse_version(env!("CARGO_PKG_VERSION")) {
        return CheckEvent::UpToDate;
    }

    let Some(asset) = release.assets.iter().find(|a| a.name == asset_name()) else {
        return CheckEvent::Failed(format!("No {} asset in release {}", asset_name(), release.tag_name));
    };

    CheckEvent::Available(UpdateInfo {
        version: release.tag_name.trim_start_matches('v').to_string(),
        asset_url: asset.browser_download_url.clone(),
    })
}

/// Downloads `info`'s asset, extracts the platform binary from it (or uses
/// it as-is on Windows, where the asset already *is* the raw exe), and
/// replaces the running executable with it via `self_replace` — takes
/// effect the next time the app is launched.
pub fn spawn_apply(info: UpdateInfo) -> Receiver<ApplyEvent> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let event = match apply(&info) {
            Ok(()) => ApplyEvent::Done,
            Err(err) => ApplyEvent::Failed(err),
        };
        let _ = tx.send(event);
    });
    rx
}

fn apply(info: &UpdateInfo) -> Result<(), String> {
    let bytes = download(&info.asset_url)?;
    let binary = extract_binary(&bytes)?;

    // `self_replace` copies this into place next to the running executable
    // itself and restores its original permissions, so it doesn't need to
    // already be executable — just needs to exist on disk somewhere.
    let temp_path = std::env::temp_dir().join(format!("rusty_comic_reader_update_{}", std::process::id()));
    std::fs::write(&temp_path, &binary).map_err(|e| e.to_string())?;
    let result = self_replace::self_replace(&temp_path);
    let _ = std::fs::remove_file(&temp_path);
    result.map_err(|e| e.to_string())?;

    // Swapping the executable in place breaks the `.app` bundle's code
    // signature — `codesign --verify` on it afterward fails with "invalid
    // Info.plist (plist or signature have been modified)", since the sealed
    // hash no longer matches. That's not just cosmetic: Finder's Open
    // With picker filters out apps that fail this check, so an
    // auto-updated Comic Reader silently drops out of "Open With" for
    // .cbz/.cb7/.cbr — re-signing (ad-hoc, same as the CI build does)
    // reseals it. Best-effort: a failure here doesn't undo the update
    // itself, which already succeeded.
    #[cfg(target_os = "macos")]
    resign_app_bundle_after_update();

    Ok(())
}

/// Re-signs the `.app` bundle containing the just-updated executable, ad-hoc
/// (`codesign --force --deep -s -`), matching how `release.yml` signs it
/// originally. No-op if `current_exe()` isn't actually inside a `.app`
/// bundle (e.g. a dev build run directly) — resigning some unrelated parent
/// directory would be actively harmful, so this only proceeds once the path
/// shape (`*.app/Contents/MacOS/<exe>`) is confirmed.
#[cfg(target_os = "macos")]
fn resign_app_bundle_after_update() {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(bundle) = app_bundle_root(&exe) else { return };
    match std::process::Command::new("codesign").args(["--force", "--deep", "-s", "-"]).arg(bundle).status() {
        Ok(status) if status.success() => {}
        Ok(status) => tracing::warn!("codesign exited with {status} while re-signing after update"),
        Err(err) => tracing::warn!("failed to run codesign while re-signing after update: {err}"),
    }
}

/// The `.app` bundle root containing `exe`, if `exe`'s path actually has the
/// shape `*.app/Contents/MacOS/<name>` — `None` otherwise (e.g. a dev build
/// run directly, where resigning some unrelated parent directory would be
/// actively harmful rather than just a no-op).
#[cfg(target_os = "macos")]
fn app_bundle_root(exe: &std::path::Path) -> Option<&std::path::Path> {
    let macos_dir = exe.parent()?;
    let contents_dir = macos_dir.parent()?;
    let bundle = contents_dir.parent()?;
    let is_bundle = macos_dir.file_name().is_some_and(|n| n == "MacOS")
        && contents_dir.file_name().is_some_and(|n| n == "Contents")
        && bundle.extension().is_some_and(|e| e == "app");
    is_bundle.then_some(bundle)
}

/// Release assets are well over ureq's default 10MB read limit.
const MAX_ASSET_SIZE: u64 = 200 * 1024 * 1024;

fn download(url: &str) -> Result<Vec<u8>, String> {
    let mut response = ureq::get(url).header("User-Agent", USER_AGENT).call().map_err(|e| e.to_string())?;
    response.body_mut().with_config().limit(MAX_ASSET_SIZE).read_to_vec().map_err(|e| e.to_string())
}

/// Pulls the platform binary out of the downloaded release asset — a zip
/// (macOS, containing the whole `.app` bundle) or tar.gz (Linux, containing
/// a folder) archive.
fn extract_binary(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if cfg!(target_os = "windows") {
        return Ok(bytes.to_vec());
    }
    if cfg!(target_os = "macos") {
        return extract_from_zip(bytes, "/Contents/MacOS/rusty_comic_reader");
    }
    extract_from_tar_gz(bytes, "/rusty_comic_reader")
}

fn extract_from_zip(bytes: &[u8], suffix: &str) -> Result<Vec<u8>, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        if file.name().ends_with(suffix) {
            let mut data = Vec::new();
            file.read_to_end(&mut data).map_err(|e| e.to_string())?;
            return Ok(data);
        }
    }
    Err(format!("No entry ending with {suffix} in the update archive"))
}

fn extract_from_tar_gz(bytes: &[u8], suffix: &str) -> Result<Vec<u8>, String> {
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive.entries().map_err(|e| e.to_string())?;
    for entry in entries {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
        if path.ends_with(suffix) {
            let mut data = Vec::new();
            entry.read_to_end(&mut data).map_err(|e| e.to_string())?;
            return Ok(data);
        }
    }
    Err(format!("No entry ending with {suffix} in the update archive"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_plain_and_v_prefixed_versions() {
        assert_eq!(parse_version("0.1.4"), (0, 1, 4));
        assert_eq!(parse_version("v0.1.4"), (0, 1, 4));
    }

    #[test]
    fn compares_versions_numerically_not_lexically() {
        // A naive string compare would put "0.1.10" before "0.1.9".
        assert!(parse_version("0.1.10") > parse_version("0.1.9"));
        assert!(parse_version("v1.0.0") > parse_version("0.9.9"));
        assert_eq!(parse_version("v0.1.4"), parse_version("0.1.4"));
    }

    #[test]
    fn extracts_the_binary_from_a_nested_zip_path() {
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("Comic Reader.app/Contents/MacOS/rusty_comic_reader", options).unwrap();
            writer.write_all(b"fake mach-o bytes").unwrap();
            writer.start_file("Comic Reader.app/Contents/Info.plist", options).unwrap();
            writer.write_all(b"<xml/>").unwrap();
            writer.finish().unwrap();
        }
        let extracted = extract_from_zip(&buf, "/Contents/MacOS/rusty_comic_reader").unwrap();
        assert_eq!(extracted, b"fake mach-o bytes");
    }

    #[test]
    fn extracts_the_binary_from_a_nested_tar_gz_path() {
        let mut gz = Vec::new();
        {
            let encoder = flate2::write::GzEncoder::new(&mut gz, flate2::Compression::default());
            let mut builder = tar::Builder::new(encoder);
            let data = b"fake elf bytes";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_cksum();
            builder.append_data(&mut header, "comic-reader/rusty_comic_reader", &data[..]).unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }
        let extracted = extract_from_tar_gz(&gz, "/rusty_comic_reader").unwrap();
        assert_eq!(extracted, b"fake elf bytes");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn app_bundle_root_matches_a_real_bundle_executable_path() {
        let exe = std::path::Path::new("/Applications/Comic Reader.app/Contents/MacOS/rusty_comic_reader");
        assert_eq!(app_bundle_root(exe), Some(std::path::Path::new("/Applications/Comic Reader.app")));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn app_bundle_root_is_none_for_a_dev_build_run_directly() {
        let exe = std::path::Path::new("/Users/dev/RustyComicReader/target/debug/rusty_comic_reader");
        assert_eq!(app_bundle_root(exe), None);
    }

    /// Hits the real GitHub API — not run by default (`cargo test`), only
    /// with `cargo test -- --ignored`, as a manual end-to-end sanity check
    /// that the request/response shape still matches what `check()` expects.
    #[test]
    #[ignore]
    fn real_check_against_github_does_not_error() {
        match check() {
            CheckEvent::Failed(err) => panic!("check() failed: {err}"),
            CheckEvent::Available(_) | CheckEvent::UpToDate => {}
        }
    }

    /// Downloads the real, currently-published release asset for this
    /// platform and extracts the binary from it — the exact code path
    /// `apply()` uses, minus the final `self_replace` call (which would
    /// overwrite the test harness's own executable). Confirms the extracted
    /// bytes look like a real, complete executable rather than e.g. an
    /// HTML error page or a truncated download. Network-dependent, so only
    /// run explicitly with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn real_download_and_extract_produces_a_valid_executable() {
        let CheckEvent::Available(info) = (match check() {
            CheckEvent::Available(info) => CheckEvent::Available(info),
            CheckEvent::UpToDate => {
                // Nothing newer than what's running — fetch the latest
                // release directly instead, just to exercise the download.
                let url = format!("https://api.github.com/repos/{REPO_OWNER}/{REPO_NAME}/releases/latest");
                let mut response = ureq::get(&url).header("User-Agent", USER_AGENT).call().unwrap();
                let body = response.body_mut().read_to_string().unwrap();
                let release: Release = serde_json::from_str(&body).unwrap();
                let asset = release.assets.iter().find(|a| a.name == asset_name()).unwrap();
                CheckEvent::Available(UpdateInfo {
                    version: release.tag_name,
                    asset_url: asset.browser_download_url.clone(),
                })
            }
            CheckEvent::Failed(err) => panic!("check() failed: {err}"),
        }) else {
            unreachable!()
        };

        let bytes = download(&info.asset_url).expect("download failed");
        assert!(bytes.len() > 10_000, "downloaded asset suspiciously small: {} bytes", bytes.len());

        let binary = extract_binary(&bytes).expect("extraction failed");
        assert!(binary.len() > 100_000, "extracted binary suspiciously small: {} bytes", binary.len());

        #[cfg(target_os = "macos")]
        assert!(
            binary.starts_with(&[0xCF, 0xFA, 0xED, 0xFE]) || binary.starts_with(&[0xCA, 0xFE, 0xBA, 0xBE]),
            "extracted file doesn't start with a Mach-O (or universal binary) magic number"
        );
        #[cfg(target_os = "linux")]
        assert!(binary.starts_with(&[0x7F, b'E', b'L', b'F']), "extracted file doesn't start with an ELF magic number");
    }
}
