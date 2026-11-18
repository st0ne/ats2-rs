//! Device identity and read-only descriptor snapshots.

use rusb::{Device, DeviceDescriptor, UsbContext};

use crate::error::Ats2Error;

/// USB vendor ID assigned to the supported adapter.
pub const SUPPORTED_VENDOR_ID: u16 = 0x16d0;
/// USB product ID assigned to the supported adapter.
pub const SUPPORTED_PRODUCT_ID: u16 = 0x0af0;

/// Returns whether the IDs match the supported adapter.
pub const fn is_supported_adapter(vendor_id: u16, product_id: u16) -> bool {
    vendor_id == SUPPORTED_VENDOR_ID && product_id == SUPPORTED_PRODUCT_ID
}

/// A detached snapshot of standard USB descriptors; contains no open handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbDeviceInfo {
    pub bus: u8,
    pub address: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub usb_version: String,
    pub device_version: String,
    pub device_class: u8,
    pub device_subclass: u8,
    pub device_protocol: u8,
    pub configurations: Vec<ConfigurationInfo>,
}

impl UsbDeviceInfo {
    pub fn is_supported_adapter(&self) -> bool {
        is_supported_adapter(self.vendor_id, self.product_id)
    }

    pub fn id_string(&self) -> String {
        format!("{:04x}:{:04x}", self.vendor_id, self.product_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationInfo {
    pub value: u8,
    pub max_power_ma: u16,
    pub self_powered: bool,
    pub remote_wakeup: bool,
    pub interfaces: Vec<InterfaceInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceInfo {
    pub number: u8,
    pub alternate_setting: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub endpoints: Vec<EndpointInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointInfo {
    pub address: u8,
    pub direction: String,
    pub transfer_type: String,
    pub max_packet_size: u16,
    pub interval: u8,
}

pub(crate) fn snapshot<T: UsbContext>(
    device: &Device<T>,
    descriptor: DeviceDescriptor,
) -> Result<UsbDeviceInfo, Ats2Error> {
    let mut configurations = Vec::new();

    for index in 0..descriptor.num_configurations() {
        let config = device.config_descriptor(index)?;
        let interfaces = config
            .interfaces()
            .flat_map(|interface| {
                interface.descriptors().map(|alternate| InterfaceInfo {
                    number: alternate.interface_number(),
                    alternate_setting: alternate.setting_number(),
                    class: alternate.class_code(),
                    subclass: alternate.sub_class_code(),
                    protocol: alternate.protocol_code(),
                    endpoints: alternate
                        .endpoint_descriptors()
                        .map(|endpoint| EndpointInfo {
                            address: endpoint.address(),
                            direction: format!("{:?}", endpoint.direction()),
                            transfer_type: format!("{:?}", endpoint.transfer_type()),
                            max_packet_size: endpoint.max_packet_size(),
                            interval: endpoint.interval(),
                        })
                        .collect(),
                })
            })
            .collect();

        configurations.push(ConfigurationInfo {
            value: config.number(),
            max_power_ma: config.max_power(),
            self_powered: config.self_powered(),
            remote_wakeup: config.remote_wakeup(),
            interfaces,
        });
    }

    let usb_version = descriptor.usb_version();
    let device_version = descriptor.device_version();

    Ok(UsbDeviceInfo {
        bus: device.bus_number(),
        address: device.address(),
        vendor_id: descriptor.vendor_id(),
        product_id: descriptor.product_id(),
        usb_version: format!("{}.{}.{}", usb_version.0, usb_version.1, usb_version.2),
        device_version: format!(
            "{}.{}.{}",
            device_version.0, device_version.1, device_version.2
        ),
        device_class: descriptor.class_code(),
        device_subclass: descriptor.sub_class_code(),
        device_protocol: descriptor.protocol_code(),
        configurations,
    })
}
