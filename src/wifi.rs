/// WiFi connection helper.
///
/// Connects to the access point configured in `cfg.toml` and returns a
/// [`BlockingWifi`] handle that keeps the connection alive for the lifetime
/// of the returned value.

use anyhow::{bail, Result};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::modem::Modem,
    nvs::EspDefaultNvsPartition,
    wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi},
};
use log::info;
use std::time::Duration;

/// Connect to the WiFi network identified by `ssid` and `password`.
///
/// Blocks until an IP address is obtained or returns an error.
/// The returned [`BlockingWifi`] must be kept alive for the duration of the
/// program — dropping it disconnects from WiFi.
pub fn connect(
    modem: Modem,
    sysloop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
    ssid: &str,
    password: &str,
) -> Result<BlockingWifi<EspWifi<'static>>> {
    if ssid.is_empty() {
        bail!(
            "WIFI_SSID is empty — copy cfg.toml.example to cfg.toml and set your credentials."
        );
    }

    let esp_wifi = EspWifi::new(modem, sysloop.clone(), Some(nvs))?;
    let mut wifi = BlockingWifi::wrap(esp_wifi, sysloop)?;

    // Choose WPA2-Personal if a password is provided, otherwise open network.
    let auth_method = if password.is_empty() {
        AuthMethod::None
    } else {
        AuthMethod::WPA2Personal
    };

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: ssid
            .try_into()
            .map_err(|_| anyhow::anyhow!("SSID too long (max 32 chars)"))?,
        password: password
            .try_into()
            .map_err(|_| anyhow::anyhow!("WiFi password too long (max 64 chars)"))?,
        auth_method,
        ..Default::default()
    }))?;

    info!("WiFi: starting…");
    wifi.start()?;

    info!("WiFi: connecting to \"{}\"…", ssid);
    wifi.connect()?;

    info!("WiFi: waiting for DHCP lease…");
    wifi.wait_netif_up()?;

    let ip = wifi.wifi().sta_netif().get_ip_info()?;
    info!("WiFi: connected — IP {}", ip.ip);

    // Keep the connection alive by sleeping briefly so the task scheduler can
    // run the WiFi background tasks before we return.
    std::thread::sleep(Duration::from_millis(100));

    Ok(wifi)
}
