use std::process::ExitCode;

use ats2_rs::{
    device::{SUPPORTED_PRODUCT_ID, SUPPORTED_VENDOR_ID, UsbDeviceInfo, is_supported_adapter},
    error::Ats2Error,
    transport::usb::enumerate_devices,
};
use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "ats2",
    version,
    about = "Read-only ATS-2 USB descriptor inspection"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List USB devices and mark the supported ATS-2 adapter.
    List,
    /// Show configurations, interfaces, and endpoints of matching adapters.
    Info,
}

fn main() -> ExitCode {
    init_tracing();

    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn run(cli: Cli) -> Result<(), Ats2Error> {
    let devices = enumerate_devices()?;
    info!(device_count = devices.len(), "USB enumeration complete");

    match cli.command {
        Command::List => print_device_list(&devices),
        Command::Info => print_supported_info(&devices),
    }
    Ok(())
}

fn print_device_list(devices: &[UsbDeviceInfo]) {
    if devices.is_empty() {
        println!("No USB devices found.");
        return;
    }

    for device in devices {
        let marker = if is_supported_adapter(device.vendor_id, device.product_id) {
            " [ATS-2 adapter]"
        } else {
            ""
        };
        println!(
            "Bus {:03} Device {:03}: ID {}{}",
            device.bus,
            device.address,
            device.id_string(),
            marker
        );
    }
}

fn print_supported_info(devices: &[UsbDeviceInfo]) {
    let mut found = false;

    for device in devices
        .iter()
        .filter(|device| device.is_supported_adapter())
    {
        found = true;
        print_device_info(device);
    }

    if !found {
        println!(
            "No supported ATS-2 adapter (VID:PID {:04x}:{:04x}) found.",
            SUPPORTED_VENDOR_ID, SUPPORTED_PRODUCT_ID
        );
    }
}

fn print_device_info(device: &UsbDeviceInfo) {
    println!(
        "ATS-2 adapter at bus {:03}, address {:03}",
        device.bus, device.address
    );
    println!("  VID:PID: {}", device.id_string());
    println!("  USB version: {}", device.usb_version);
    println!("  Device version: {}", device.device_version);
    println!(
        "  Device class/subclass/protocol: {:02x}/{:02x}/{:02x}",
        device.device_class, device.device_subclass, device.device_protocol
    );

    for config in &device.configurations {
        println!(
            "  Configuration {}: {} mA, self-powered={}, remote-wakeup={}",
            config.value, config.max_power_ma, config.self_powered, config.remote_wakeup
        );
        for interface in &config.interfaces {
            println!(
                "    Interface {} alt {}: class/subclass/protocol {:02x}/{:02x}/{:02x}",
                interface.number,
                interface.alternate_setting,
                interface.class,
                interface.subclass,
                interface.protocol
            );
            for endpoint in &interface.endpoints {
                println!(
                    "      Endpoint 0x{:02x}: {:?} {}, max packet {}, interval {}",
                    endpoint.address,
                    endpoint.direction,
                    endpoint.transfer_type,
                    endpoint.max_packet_size,
                    endpoint.interval
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command};
    use clap::Parser;

    #[test]
    fn parses_list_subcommand() {
        let cli = Cli::try_parse_from(["ats2", "list"]).expect("list should parse");
        assert!(matches!(cli.command, Command::List));
    }

    #[test]
    fn parses_info_subcommand() {
        let cli = Cli::try_parse_from(["ats2", "info"]).expect("info should parse");
        assert!(matches!(cli.command, Command::Info));
    }
}
