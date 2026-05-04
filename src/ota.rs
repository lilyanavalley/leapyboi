/// OTA (over-the-air) firmware update helper.
///
/// # Flow
///
/// 1. `check_and_update` fetches a tiny version file from the server.
/// 2. If the remote version differs from the compiled-in `CARGO_PKG_VERSION`
///    the full firmware binary is downloaded and written to the inactive OTA
///    flash partition via `esp_idf_svc::ota::EspOta`.
/// 3. On success the device restarts and boots the new firmware.
/// 4. On any error the function returns so the caller can log the problem and
///    keep running the current firmware.

use anyhow::{anyhow, Result};
use embedded_svc::http::{client::Client as HttpClient, Method};
use embedded_svc::io::Read;
use esp_idf_svc::http::client::{Configuration as HttpConfig, EspHttpConnection};
use esp_idf_svc::ota::EspOta;
use log::{error, info, warn};

/// Version string embedded at compile time from `Cargo.toml`.
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

// ── Public API ────────────────────────────────────────────────────────────────

/// Fetch the remote version file and apply an OTA update if a newer firmware
/// is available.
///
/// Returns `Ok(false)` when the firmware is already up to date.
/// Returns `Ok(true)` if an update was initiated — however in practice the
/// device will have restarted before this value is returned to the caller.
/// Returns `Err` if the version check or download failed; the caller can log
/// the error and continue running the current firmware.
pub fn check_and_update(version_url: &str, firmware_url: &str) -> Result<bool> {
    if version_url.is_empty() || firmware_url.is_empty() {
        warn!("OTA: ota_firmware_url / ota_version_url not configured — skipping");
        return Ok(false);
    }

    info!("OTA: running v{} — checking {}", CURRENT_VERSION, version_url);

    let remote_version = match fetch_text(version_url) {
        Ok(v) => v,
        Err(e) => {
            warn!("OTA: could not fetch version file — {e:#}");
            return Ok(false);
        }
    };
    let remote_version = remote_version.trim().to_owned();

    if remote_version == CURRENT_VERSION {
        info!("OTA: firmware is up to date (v{})", CURRENT_VERSION);
        return Ok(false);
    }

    info!(
        "OTA: update available v{} → v{} — downloading…",
        CURRENT_VERSION, remote_version
    );

    apply_update(firmware_url)
}

/// Download firmware from `firmware_url` and apply it as an OTA update.
///
/// On success the device restarts and **this function does not return**.
/// On failure an `Err` is returned so the caller can log the problem.
pub fn apply_update(firmware_url: &str) -> Result<bool> {
    if firmware_url.is_empty() {
        return Err(anyhow!("OTA: ota_firmware_url is not configured"));
    }

    info!("OTA: downloading from {}", firmware_url);

    let mut ota = EspOta::new().map_err(|e| anyhow!("EspOta::new failed: {e:?}"))?;
    let mut update = ota
        .begin()
        .map_err(|e| anyhow!("EspOta::begin failed: {e:?}"))?;

    let result = stream_firmware(firmware_url, &mut update);

    match result {
        Ok(total) => {
            info!("OTA: {} bytes written — completing update…", total);
            update
                .complete()
                .map_err(|e| anyhow!("EspOtaUpdate::complete failed: {e:?}"))?;
            info!("OTA: update complete — restarting device…");
            // Safety: we are intentionally restarting the MCU after a
            // successful OTA.  No destructors need to run.
            unsafe { esp_idf_svc::sys::esp_restart() };
        }
        Err(e) => {
            error!("OTA: download error — aborting: {e:#}");
            // Best-effort abort; ignore secondary error.
            let _ = update.abort();
            return Err(e);
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Stream firmware binary from `url` into `update`, returning the total number
/// of bytes written.
fn stream_firmware(url: &str, update: &mut esp_idf_svc::ota::EspOtaUpdate<'_>) -> Result<usize> {
    let mut client = HttpClient::wrap(http_connection()?);

    let request = client
        .request(Method::Get, url, &[])
        .map_err(|e| anyhow!("HTTP GET request failed: {e:?}"))?;
    let mut response = request
        .submit()
        .map_err(|e| anyhow!("HTTP submit failed: {e:?}"))?;

    let status = response.status();
    if status != 200 {
        return Err(anyhow!(
            "OTA: server returned HTTP {} for firmware URL",
            status
        ));
    }

    let mut buf = [0u8; 4096];
    let mut total = 0usize;

    loop {
        let n = response
            .read(&mut buf)
            .map_err(|e| anyhow!("HTTP read error: {e:?}"))?;
        if n == 0 {
            break;
        }
        update
            .write(&buf[..n])
            .map_err(|e| anyhow!("OTA write error: {e:?}"))?;
        total += n;
    }

    Ok(total)
}

/// Fetch the body of a small text resource (e.g. version.txt) and return it
/// as a `String`.
fn fetch_text(url: &str) -> Result<String> {
    let mut client = HttpClient::wrap(http_connection()?);

    let request = client
        .request(Method::Get, url, &[])
        .map_err(|e| anyhow!("HTTP GET request failed: {e:?}"))?;
    let mut response = request
        .submit()
        .map_err(|e| anyhow!("HTTP submit failed: {e:?}"))?;

    let status = response.status();
    if status != 200 {
        return Err(anyhow!("HTTP {} fetching {}", status, url));
    }

    // Version string is at most a few dozen bytes.
    let mut buf = [0u8; 64];
    let n = response
        .read(&mut buf)
        .map_err(|e| anyhow!("HTTP read error: {e:?}"))?;

    String::from_utf8(buf[..n].to_vec()).map_err(|e| anyhow!("Non-UTF-8 version response: {e}"))
}

/// Create an HTTPS-capable `EspHttpConnection` using the ESP-IDF bundled CA
/// certificates so that GitHub's TLS certificate can be verified.
fn http_connection() -> Result<EspHttpConnection> {
    EspHttpConnection::new(&HttpConfig {
        use_global_ca_store: true,
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        ..Default::default()
    })
    .map_err(|e| anyhow!("failed to create HTTP connection: {e:?}"))
}
