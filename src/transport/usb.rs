//! Safe USB discovery plus explicitly invoked, descriptor-validated bulk transport.

use rusb::{Context, UsbContext};
use thiserror::Error;
use tracing::debug;

use crate::{
    device::{UsbDeviceInfo, snapshot},
    error::Ats2Error,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndpointDirection {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndpointTransferType {
    Bulk,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EndpointDescriptorSnapshot {
    address: u8,
    direction: EndpointDirection,
    transfer_type: EndpointTransferType,
    max_packet_size: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InterfaceDescriptorSnapshot {
    configuration_value: u8,
    interface_number: u8,
    alternate_setting: u8,
    endpoints: Vec<EndpointDescriptorSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BulkEndpoint {
    pub address: u8,
    pub max_packet_size: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BulkEndpointPair {
    pub out: BulkEndpoint,
    pub input: BulkEndpoint,
}

fn validate_target_interface(
    descriptor: &InterfaceDescriptorSnapshot,
) -> Result<BulkEndpointPair, UsbTransportError> {
    if descriptor.configuration_value != 1
        || descriptor.interface_number != 0
        || descriptor.alternate_setting != 0
    {
        return Err(UsbTransportError::InvalidTargetInterface {
            configuration_value: descriptor.configuration_value,
            interface_number: descriptor.interface_number,
            alternate_setting: descriptor.alternate_setting,
        });
    }

    fn endpoint(
        descriptors: &[EndpointDescriptorSnapshot],
        address: u8,
        direction: EndpointDirection,
    ) -> Result<BulkEndpoint, UsbTransportError> {
        let mut matches = descriptors
            .iter()
            .filter(|endpoint| endpoint.address == address);
        let Some(endpoint) = matches.next() else {
            return Err(UsbTransportError::MissingEndpoint { address });
        };
        if matches.next().is_some() {
            return Err(UsbTransportError::DuplicateEndpoint { address });
        }
        if endpoint.direction != direction {
            return Err(UsbTransportError::InvalidEndpointDirection { address });
        }
        if endpoint.transfer_type != EndpointTransferType::Bulk {
            return Err(UsbTransportError::InvalidEndpointType { address });
        }
        if !matches!(endpoint.max_packet_size, 8 | 16 | 32 | 64 | 512) {
            return Err(UsbTransportError::InvalidMaxPacketSize {
                address,
                max_packet_size: endpoint.max_packet_size,
            });
        }
        Ok(BulkEndpoint {
            address,
            max_packet_size: endpoint.max_packet_size,
        })
    }

    Ok(BulkEndpointPair {
        out: endpoint(&descriptor.endpoints, 0x06, EndpointDirection::Out)?,
        input: endpoint(&descriptor.endpoints, 0x82, EndpointDirection::In)?,
    })
}

/// Enumerate USB devices and read their device/configuration/interface/endpoint descriptors.
///
/// This function does not open a device handle, set a configuration, claim an interface,
/// or issue class/vendor-specific transfers.
pub fn enumerate_devices() -> Result<Vec<UsbDeviceInfo>, Ats2Error> {
    let context = Context::new()?;
    let devices = context.devices()?;
    let mut result = Vec::with_capacity(devices.len());

    for device in devices.iter() {
        let descriptor = device.device_descriptor()?;
        debug!(
            bus = device.bus_number(),
            address = device.address(),
            vendor_id = format_args!("{:04x}", descriptor.vendor_id()),
            product_id = format_args!("{:04x}", descriptor.product_id()),
            "USB device enumerated"
        );
        result.push(snapshot(&device, descriptor)?);
    }

    Ok(result)
}

/// Default timeout used by [`open_supported_adapter`].
pub const DEFAULT_USB_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const MAX_USB_BULK_TRANSFER_LEN: usize = 0x2000;
const TARGET_VENDOR_ID: u16 = 0x16d0;
const TARGET_PRODUCT_ID: u16 = 0x0af0;
const TARGET_CONFIGURATION: u8 = 1;
const TARGET_INTERFACE: u8 = 0;
const TARGET_ALTERNATE_SETTING: u8 = 0;
const GET_INTERFACE_REQUEST: u8 = 0x0a;

/// Direction of a USB bulk transfer reported by [`UsbTransportError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkDirection {
    /// Device-to-host bulk IN.
    In,
    /// Host-to-device bulk OUT.
    Out,
}

#[derive(Debug, Error)]
pub enum UsbTransportError {
    #[error(
        "configuration {configuration_value}, interface {interface_number}, alternate setting {alternate_setting} is not the required 1/0/0 target"
    )]
    InvalidTargetInterface {
        configuration_value: u8,
        interface_number: u8,
        alternate_setting: u8,
    },
    #[error("required bulk endpoint 0x{address:02x} is missing")]
    MissingEndpoint { address: u8 },
    #[error("bulk endpoint 0x{address:02x} appears more than once")]
    DuplicateEndpoint { address: u8 },
    #[error("endpoint 0x{address:02x} has the wrong direction")]
    InvalidEndpointDirection { address: u8 },
    #[error("endpoint 0x{address:02x} is not a bulk endpoint")]
    InvalidEndpointType { address: u8 },
    #[error("endpoint 0x{address:02x} has unsupported maximum packet size {max_packet_size}")]
    InvalidMaxPacketSize { address: u8, max_packet_size: u16 },
    #[error("USB operation failed: {0}")]
    Usb(#[from] rusb::Error),
    #[error("no USB device with VID:PID {TARGET_VENDOR_ID:04X}:{TARGET_PRODUCT_ID:04X} was found")]
    DeviceNotFound,
    #[error("found {count} ATS-2 adapters; refusing to choose one implicitly")]
    MultipleDevices { count: usize },
    #[error("the ATS-2 adapter has no configuration {TARGET_CONFIGURATION}")]
    ConfigurationNotFound,
    #[error(
        "configuration {TARGET_CONFIGURATION} has no interface {TARGET_INTERFACE}, alternate setting {TARGET_ALTERNATE_SETTING}"
    )]
    InterfaceSettingNotFound,
    #[error(
        "configuration {TARGET_CONFIGURATION}, interface {TARGET_INTERFACE}, alternate setting {TARGET_ALTERNATE_SETTING} appears more than once"
    )]
    DuplicateInterfaceSetting,
    #[error("USB timeout {milliseconds} ms is invalid; use 1..={max_milliseconds} ms")]
    InvalidTimeout {
        milliseconds: u128,
        max_milliseconds: u128,
    },
    #[error(
        "active USB configuration is {actual}; expected {TARGET_CONFIGURATION}. No configuration was changed"
    )]
    ActiveConfigurationMismatch { actual: u8 },
    #[error("cannot claim USB interface {interface}: {source}; no kernel driver was detached")]
    InterfaceClaimFailed {
        interface: u8,
        #[source]
        source: rusb::Error,
    },
    #[error("GET_INTERFACE for interface {interface} returned {actual} byte(s), expected 1")]
    InvalidInterfaceSettingResponse { interface: u8, actual: usize },
    #[error(
        "active alternate setting is {actual}; expected {TARGET_ALTERNATE_SETTING}. No setting was changed"
    )]
    AlternateSettingMismatch { actual: u8 },
    #[error("bulk {direction:?} transfer timed out after {timeout:?}")]
    TransferTimeout {
        direction: BulkDirection,
        timeout: std::time::Duration,
    },
    #[error("bulk {direction:?} transfer failed: {source}")]
    TransferFailed {
        direction: BulkDirection,
        #[source]
        source: rusb::Error,
    },
    #[error("bulk {direction:?} transfer was short: expected {expected} byte(s), got {actual}")]
    ShortTransfer {
        direction: BulkDirection,
        expected: usize,
        actual: usize,
    },
    #[error(
        "bulk {direction:?} transfer returned impossible byte count {actual} for a {requested}-byte buffer"
    )]
    InvalidTransferCount {
        direction: BulkDirection,
        requested: usize,
        actual: usize,
    },
    #[error(
        "bulk {direction:?} length {length} is invalid; expected 1..={MAX_USB_BULK_TRANSFER_LEN} bytes"
    )]
    InvalidTransferLength {
        direction: BulkDirection,
        length: usize,
    },
}

fn validate_timeout(timeout: std::time::Duration) -> Result<(), UsbTransportError> {
    let milliseconds = timeout.as_millis();
    if milliseconds == 0 || milliseconds > u128::from(u32::MAX) {
        return Err(UsbTransportError::InvalidTimeout {
            milliseconds,
            max_milliseconds: u128::from(u32::MAX),
        });
    }
    Ok(())
}

fn timeout_error(
    direction: BulkDirection,
    timeout: std::time::Duration,
    source: rusb::Error,
) -> UsbTransportError {
    if source == rusb::Error::Timeout {
        UsbTransportError::TransferTimeout { direction, timeout }
    } else {
        UsbTransportError::TransferFailed { direction, source }
    }
}

trait UsbHandleOps {
    fn active_configuration(&self) -> rusb::Result<u8>;
    fn claim_interface(&self, interface: u8) -> rusb::Result<()>;
    fn read_interface_alt(
        &self,
        interface: u8,
        value: &mut [u8; 1],
        timeout: std::time::Duration,
    ) -> rusb::Result<usize>;
    fn release_interface(&self, interface: u8) -> rusb::Result<()>;
    fn write_bulk(
        &self,
        endpoint: u8,
        bytes: &[u8],
        timeout: std::time::Duration,
    ) -> rusb::Result<usize>;
    fn read_bulk(
        &self,
        endpoint: u8,
        bytes: &mut [u8],
        timeout: std::time::Duration,
    ) -> rusb::Result<usize>;
}

impl UsbHandleOps for rusb::DeviceHandle<Context> {
    fn active_configuration(&self) -> rusb::Result<u8> {
        rusb::DeviceHandle::active_configuration(self)
    }

    fn claim_interface(&self, interface: u8) -> rusb::Result<()> {
        rusb::DeviceHandle::claim_interface(self, interface)
    }

    fn read_interface_alt(
        &self,
        interface: u8,
        value: &mut [u8; 1],
        timeout: std::time::Duration,
    ) -> rusb::Result<usize> {
        // Read-only USB standard GET_INTERFACE; no vendor request or setting change.
        let request_type = rusb::request_type(
            rusb::Direction::In,
            rusb::RequestType::Standard,
            rusb::Recipient::Interface,
        );
        rusb::DeviceHandle::read_control(
            self,
            request_type,
            GET_INTERFACE_REQUEST,
            0,
            u16::from(interface),
            value,
            timeout,
        )
    }

    fn release_interface(&self, interface: u8) -> rusb::Result<()> {
        rusb::DeviceHandle::release_interface(self, interface)
    }

    fn write_bulk(
        &self,
        endpoint: u8,
        bytes: &[u8],
        timeout: std::time::Duration,
    ) -> rusb::Result<usize> {
        rusb::DeviceHandle::write_bulk(self, endpoint, bytes, timeout)
    }

    fn read_bulk(
        &self,
        endpoint: u8,
        bytes: &mut [u8],
        timeout: std::time::Duration,
    ) -> rusb::Result<usize> {
        rusb::DeviceHandle::read_bulk(self, endpoint, bytes, timeout)
    }
}

struct ClaimedInterface<H: UsbHandleOps> {
    handle: H,
    interface: u8,
    claimed: bool,
}

impl<H: UsbHandleOps> ClaimedInterface<H> {
    fn release(&mut self) -> Result<(), UsbTransportError> {
        if self.claimed {
            self.claimed = false;
            self.handle
                .release_interface(self.interface)
                .map_err(UsbTransportError::Usb)?;
            tracing::info!(interface = self.interface, "USB interface released");
        }
        Ok(())
    }
}

impl<H: UsbHandleOps> Drop for ClaimedInterface<H> {
    fn drop(&mut self) {
        if self.claimed {
            self.claimed = false;
            if let Err(error) = self.handle.release_interface(self.interface) {
                tracing::warn!(interface = self.interface, %error, "failed to release USB interface during drop");
            } else {
                tracing::info!(interface = self.interface, "USB interface released on drop");
            }
        }
    }
}

fn claim_usb_interface<H: UsbHandleOps>(
    handle: H,
    timeout: std::time::Duration,
) -> Result<ClaimedInterface<H>, UsbTransportError> {
    validate_timeout(timeout)?;
    let active_configuration = handle.active_configuration()?;
    if active_configuration != TARGET_CONFIGURATION {
        return Err(UsbTransportError::ActiveConfigurationMismatch {
            actual: active_configuration,
        });
    }
    handle.claim_interface(TARGET_INTERFACE).map_err(|source| {
        UsbTransportError::InterfaceClaimFailed {
            interface: TARGET_INTERFACE,
            source,
        }
    })?;

    let claimed = ClaimedInterface {
        handle,
        interface: TARGET_INTERFACE,
        claimed: true,
    };
    let mut alternate_setting = [0_u8; 1];
    let actual =
        claimed
            .handle
            .read_interface_alt(TARGET_INTERFACE, &mut alternate_setting, timeout)?;
    if actual != alternate_setting.len() {
        return Err(UsbTransportError::InvalidInterfaceSettingResponse {
            interface: TARGET_INTERFACE,
            actual,
        });
    }
    if alternate_setting[0] != TARGET_ALTERNATE_SETTING {
        return Err(UsbTransportError::AlternateSettingMismatch {
            actual: alternate_setting[0],
        });
    }
    Ok(claimed)
}

/// Explicit interface-level bulk I/O contract, independent of APIB framing.
pub trait BulkTransport {
    /// Maximum packet size read from the EP6 OUT descriptor.
    fn out_max_packet_size(&self) -> u16;
    /// Maximum packet size read from the EP2 IN descriptor.
    fn in_max_packet_size(&self) -> u16;
    /// Write one complete bulk OUT buffer, rejecting short writes.
    fn bulk_out(&mut self, bytes: &[u8]) -> Result<(), UsbTransportError>;
    /// Read up to `length` bytes and return the actual received byte count as a vector.
    /// Short reads are preserved for protocol-level response-span validation.
    fn bulk_in(&mut self, length: usize) -> Result<Vec<u8>, UsbTransportError>;
}

struct CheckedBulkTransport<H: UsbHandleOps> {
    interface: ClaimedInterface<H>,
    endpoints: BulkEndpointPair,
    timeout: std::time::Duration,
}

impl<H: UsbHandleOps> CheckedBulkTransport<H> {
    fn new(
        handle: H,
        endpoints: BulkEndpointPair,
        timeout: std::time::Duration,
    ) -> Result<Self, UsbTransportError> {
        validate_timeout(timeout)?;
        let interface = claim_usb_interface(handle, timeout)?;
        Ok(Self {
            interface,
            endpoints,
            timeout,
        })
    }

    fn release(&mut self) -> Result<(), UsbTransportError> {
        self.interface.release()
    }
}

impl<H: UsbHandleOps> BulkTransport for CheckedBulkTransport<H> {
    fn out_max_packet_size(&self) -> u16 {
        self.endpoints.out.max_packet_size
    }

    fn in_max_packet_size(&self) -> u16 {
        self.endpoints.input.max_packet_size
    }

    fn bulk_out(&mut self, bytes: &[u8]) -> Result<(), UsbTransportError> {
        if bytes.is_empty() || bytes.len() > MAX_USB_BULK_TRANSFER_LEN {
            return Err(UsbTransportError::InvalidTransferLength {
                direction: BulkDirection::Out,
                length: bytes.len(),
            });
        }
        tracing::debug!(endpoint = self.endpoints.out.address, requested = bytes.len(), timeout = ?self.timeout, "bulk OUT transfer started");
        let actual = self
            .interface
            .handle
            .write_bulk(self.endpoints.out.address, bytes, self.timeout)
            .map_err(|source| timeout_error(BulkDirection::Out, self.timeout, source))?;
        if actual > bytes.len() {
            return Err(UsbTransportError::InvalidTransferCount {
                direction: BulkDirection::Out,
                requested: bytes.len(),
                actual,
            });
        }
        if actual != bytes.len() {
            return Err(UsbTransportError::ShortTransfer {
                direction: BulkDirection::Out,
                expected: bytes.len(),
                actual,
            });
        }
        tracing::debug!(
            endpoint = self.endpoints.out.address,
            actual,
            "bulk OUT transfer completed"
        );
        Ok(())
    }

    fn bulk_in(&mut self, length: usize) -> Result<Vec<u8>, UsbTransportError> {
        if length == 0 || length > MAX_USB_BULK_TRANSFER_LEN {
            return Err(UsbTransportError::InvalidTransferLength {
                direction: BulkDirection::In,
                length,
            });
        }
        let mut bytes = vec![0_u8; length];
        tracing::debug!(endpoint = self.endpoints.input.address, requested = length, timeout = ?self.timeout, "bulk IN transfer started");
        let actual = self
            .interface
            .handle
            .read_bulk(self.endpoints.input.address, &mut bytes, self.timeout)
            .map_err(|source| timeout_error(BulkDirection::In, self.timeout, source))?;
        if actual > bytes.len() {
            return Err(UsbTransportError::InvalidTransferCount {
                direction: BulkDirection::In,
                requested: bytes.len(),
                actual,
            });
        }
        bytes.truncate(actual);
        tracing::debug!(
            endpoint = self.endpoints.input.address,
            requested = length,
            actual,
            "bulk IN transfer completed"
        );
        Ok(bytes)
    }
}

/// Opened ATS-2 USB handle. Opening only discovers descriptors and opens the handle;
/// it does not claim an interface or perform bulk/APIB communication.
pub struct OpenedUsbDevice {
    handle: rusb::DeviceHandle<Context>,
    endpoints: BulkEndpointPair,
    timeout: std::time::Duration,
}

impl OpenedUsbDevice {
    /// Validated endpoint addresses and maximum packet sizes from configuration 1/interface 0/alt 0.
    pub fn endpoints(&self) -> BulkEndpointPair {
        self.endpoints
    }

    /// Claim interface 0 only after an explicit caller request; no configuration or alt setting is changed.
    pub fn claim_interface(self) -> Result<ClaimedUsbDevice, UsbTransportError> {
        let inner = CheckedBulkTransport::new(self.handle, self.endpoints, self.timeout)?;
        tracing::info!(
            interface = TARGET_INTERFACE,
            alternate_setting = TARGET_ALTERNATE_SETTING,
            "ATS-2 USB interface claimed"
        );
        Ok(ClaimedUsbDevice { inner })
    }
}

/// Claimed adapter interface. Bulk transfers occur only when its `BulkTransport` methods are called.
pub struct ClaimedUsbDevice {
    inner: CheckedBulkTransport<rusb::DeviceHandle<Context>>,
}

impl ClaimedUsbDevice {
    /// Endpoint addresses and descriptor-derived packet sizes.
    pub fn endpoints(&self) -> BulkEndpointPair {
        self.inner.endpoints
    }

    /// Release the interface early; dropping this value also releases it.
    pub fn release(mut self) -> Result<(), UsbTransportError> {
        self.inner.release()
    }
}

impl BulkTransport for ClaimedUsbDevice {
    fn out_max_packet_size(&self) -> u16 {
        self.inner.out_max_packet_size()
    }

    fn in_max_packet_size(&self) -> u16 {
        self.inner.in_max_packet_size()
    }

    fn bulk_out(&mut self, bytes: &[u8]) -> Result<(), UsbTransportError> {
        self.inner.bulk_out(bytes)
    }

    fn bulk_in(&mut self, length: usize) -> Result<Vec<u8>, UsbTransportError> {
        self.inner.bulk_in(length)
    }
}

/// Open the unique supported device using a 2-second timeout. No interface is claimed.
pub fn open_supported_adapter() -> Result<OpenedUsbDevice, UsbTransportError> {
    open_supported_adapter_with_timeout(DEFAULT_USB_TIMEOUT)
}

/// Open the unique supported device with a caller-selected transfer/verification timeout.
pub fn open_supported_adapter_with_timeout(
    timeout: std::time::Duration,
) -> Result<OpenedUsbDevice, UsbTransportError> {
    validate_timeout(timeout)?;
    let context = Context::new()?;
    let devices = context.devices()?;
    let mut matching_device = None;
    let mut match_count = 0;
    for device in devices.iter() {
        let descriptor = device.device_descriptor()?;
        if crate::device::is_supported_adapter(descriptor.vendor_id(), descriptor.product_id()) {
            match_count += 1;
            if matching_device.is_none() {
                matching_device = Some(device);
            }
        }
    }
    if match_count == 0 {
        return Err(UsbTransportError::DeviceNotFound);
    }
    if match_count > 1 {
        return Err(UsbTransportError::MultipleDevices { count: match_count });
    }
    let device = matching_device.ok_or(UsbTransportError::DeviceNotFound)?;
    let descriptor = device.device_descriptor()?;
    let endpoints = read_supported_endpoints(&device, descriptor.num_configurations())?;
    tracing::info!(
        bus = device.bus_number(),
        address = device.address(),
        vendor_id = TARGET_VENDOR_ID,
        product_id = TARGET_PRODUCT_ID,
        "opening ATS-2 USB device handle without claiming the interface"
    );
    let handle = device.open()?;
    Ok(OpenedUsbDevice {
        handle,
        endpoints,
        timeout,
    })
}

fn read_supported_endpoints(
    device: &rusb::Device<Context>,
    configuration_count: u8,
) -> Result<BulkEndpointPair, UsbTransportError> {
    for index in 0..configuration_count {
        let configuration = device.config_descriptor(index)?;
        if configuration.number() != TARGET_CONFIGURATION {
            continue;
        }
        let mut target = None;
        for interface in configuration.interfaces() {
            for setting in interface.descriptors() {
                if setting.interface_number() == TARGET_INTERFACE
                    && setting.setting_number() == TARGET_ALTERNATE_SETTING
                {
                    if target.is_some() {
                        return Err(UsbTransportError::DuplicateInterfaceSetting);
                    }
                    let endpoints = setting
                        .endpoint_descriptors()
                        .map(|endpoint| EndpointDescriptorSnapshot {
                            address: endpoint.address(),
                            direction: match endpoint.direction() {
                                rusb::Direction::In => EndpointDirection::In,
                                rusb::Direction::Out => EndpointDirection::Out,
                            },
                            transfer_type: match endpoint.transfer_type() {
                                rusb::TransferType::Bulk => EndpointTransferType::Bulk,
                                _ => EndpointTransferType::Other,
                            },
                            max_packet_size: endpoint.max_packet_size(),
                        })
                        .collect();
                    target = Some(InterfaceDescriptorSnapshot {
                        configuration_value: configuration.number(),
                        interface_number: setting.interface_number(),
                        alternate_setting: setting.setting_number(),
                        endpoints,
                    });
                }
            }
        }
        let target = target.ok_or(UsbTransportError::InterfaceSettingNotFound)?;
        return validate_target_interface(&target);
    }
    Err(UsbTransportError::ConfigurationNotFound)
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc, time::Duration};

    use super::{
        BulkDirection, BulkEndpoint, BulkEndpointPair, BulkTransport, CheckedBulkTransport,
        EndpointDescriptorSnapshot, EndpointDirection, EndpointTransferType,
        InterfaceDescriptorSnapshot, UsbHandleOps, UsbTransportError, validate_target_interface,
    };

    #[test]
    fn validates_the_target_bulk_endpoint_pair_from_alt_zero_descriptors() {
        let descriptor = InterfaceDescriptorSnapshot {
            configuration_value: 1,
            interface_number: 0,
            alternate_setting: 0,
            endpoints: vec![
                EndpointDescriptorSnapshot {
                    address: 0x06,
                    direction: EndpointDirection::Out,
                    transfer_type: EndpointTransferType::Bulk,
                    max_packet_size: 512,
                },
                EndpointDescriptorSnapshot {
                    address: 0x82,
                    direction: EndpointDirection::In,
                    transfer_type: EndpointTransferType::Bulk,
                    max_packet_size: 512,
                },
            ],
        };

        let pair = validate_target_interface(&descriptor).unwrap();
        assert_eq!(pair.out.address, 0x06);
        assert_eq!(pair.out.max_packet_size, 512);
        assert_eq!(pair.input.address, 0x82);
        assert_eq!(pair.input.max_packet_size, 512);
    }

    #[test]
    fn rejects_a_missing_or_misconfigured_target_endpoint() {
        let mut descriptor = target_descriptor();
        descriptor.endpoints[0].address = 0x86;
        assert!(matches!(
            validate_target_interface(&descriptor),
            Err(super::UsbTransportError::MissingEndpoint { address: 0x06 })
        ));

        let mut descriptor = target_descriptor();
        descriptor.endpoints[0].transfer_type = EndpointTransferType::Other;
        assert!(matches!(
            validate_target_interface(&descriptor),
            Err(super::UsbTransportError::InvalidEndpointType { address: 0x06 })
        ));
    }

    #[test]
    fn rejects_a_non_usb2_bulk_packet_size_and_wrong_target_setting() {
        let mut descriptor = target_descriptor();
        descriptor.endpoints[0].max_packet_size = 100;
        assert!(matches!(
            validate_target_interface(&descriptor),
            Err(super::UsbTransportError::InvalidMaxPacketSize {
                address: 0x06,
                max_packet_size: 100
            })
        ));

        let mut descriptor = target_descriptor();
        descriptor.alternate_setting = 1;
        assert!(matches!(
            validate_target_interface(&descriptor),
            Err(super::UsbTransportError::InvalidTargetInterface {
                configuration_value: 1,
                interface_number: 0,
                alternate_setting: 1
            })
        ));
    }

    fn target_descriptor() -> InterfaceDescriptorSnapshot {
        InterfaceDescriptorSnapshot {
            configuration_value: 1,
            interface_number: 0,
            alternate_setting: 0,
            endpoints: vec![
                EndpointDescriptorSnapshot {
                    address: 0x06,
                    direction: EndpointDirection::Out,
                    transfer_type: EndpointTransferType::Bulk,
                    max_packet_size: 512,
                },
                EndpointDescriptorSnapshot {
                    address: 0x82,
                    direction: EndpointDirection::In,
                    transfer_type: EndpointTransferType::Bulk,
                    max_packet_size: 512,
                },
            ],
        }
    }

    #[derive(Default)]
    struct MockState {
        active_configuration: u8,
        alternate_setting: u8,
        fail_claim: bool,
        write_timeout: bool,
        read_timeout: bool,
        write_count: Option<usize>,
        read_count: Option<usize>,
        read_data: Vec<u8>,
        claim_calls: usize,
        alternate_queries: usize,
        release_calls: usize,
        drop_calls: usize,
        bulk_write_calls: usize,
        bulk_read_calls: usize,
    }

    fn mock_state() -> Rc<RefCell<MockState>> {
        let state = Rc::new(RefCell::new(MockState::default()));
        state.borrow_mut().active_configuration = 1;
        state
    }

    struct MockHandle(Rc<RefCell<MockState>>);

    impl Drop for MockHandle {
        fn drop(&mut self) {
            self.0.borrow_mut().drop_calls += 1;
        }
    }

    impl UsbHandleOps for MockHandle {
        fn active_configuration(&self) -> rusb::Result<u8> {
            Ok(self.0.borrow().active_configuration)
        }

        fn claim_interface(&self, _interface: u8) -> rusb::Result<()> {
            let mut state = self.0.borrow_mut();
            state.claim_calls += 1;
            if state.fail_claim {
                Err(rusb::Error::Access)
            } else {
                Ok(())
            }
        }

        fn read_interface_alt(
            &self,
            _interface: u8,
            value: &mut [u8; 1],
            _timeout: Duration,
        ) -> rusb::Result<usize> {
            let mut state = self.0.borrow_mut();
            state.alternate_queries += 1;
            value[0] = state.alternate_setting;
            Ok(1)
        }

        fn release_interface(&self, _interface: u8) -> rusb::Result<()> {
            self.0.borrow_mut().release_calls += 1;
            Ok(())
        }

        fn write_bulk(
            &self,
            _endpoint: u8,
            bytes: &[u8],
            _timeout: Duration,
        ) -> rusb::Result<usize> {
            let mut state = self.0.borrow_mut();
            state.bulk_write_calls += 1;
            if state.write_timeout {
                return Err(rusb::Error::Timeout);
            }
            Ok(state.write_count.unwrap_or(bytes.len()))
        }

        fn read_bulk(
            &self,
            _endpoint: u8,
            bytes: &mut [u8],
            _timeout: Duration,
        ) -> rusb::Result<usize> {
            let mut state = self.0.borrow_mut();
            state.bulk_read_calls += 1;
            if state.read_timeout {
                return Err(rusb::Error::Timeout);
            }
            let actual = state
                .read_count
                .unwrap_or(state.read_data.len())
                .min(bytes.len())
                .min(state.read_data.len());
            bytes[..actual].copy_from_slice(&state.read_data[..actual]);
            Ok(actual)
        }
    }

    fn mock_endpoints() -> BulkEndpointPair {
        BulkEndpointPair {
            out: BulkEndpoint {
                address: 0x06,
                max_packet_size: 512,
            },
            input: BulkEndpoint {
                address: 0x82,
                max_packet_size: 512,
            },
        }
    }

    #[test]
    fn interface_claim_failure_is_contextual_and_does_not_release_an_unclaimed_interface() {
        let state = mock_state();
        state.borrow_mut().fail_claim = true;
        let result = CheckedBulkTransport::new(
            MockHandle(Rc::clone(&state)),
            mock_endpoints(),
            Duration::from_secs(1),
        );
        assert!(matches!(
            result,
            Err(UsbTransportError::InterfaceClaimFailed { interface: 0, .. })
        ));
        assert_eq!(state.borrow().claim_calls, 1);
        assert_eq!(state.borrow().release_calls, 0);
        assert_eq!(state.borrow().drop_calls, 1);
    }

    #[test]
    fn claim_is_separate_from_bulk_io_and_drop_releases_resources() {
        let state = mock_state();
        let transport = CheckedBulkTransport::new(
            MockHandle(Rc::clone(&state)),
            mock_endpoints(),
            Duration::from_secs(1),
        )
        .unwrap();
        {
            let state = state.borrow();
            assert_eq!(state.claim_calls, 1);
            assert_eq!(state.alternate_queries, 1);
            assert_eq!(state.bulk_write_calls, 0);
            assert_eq!(state.bulk_read_calls, 0);
        }
        drop(transport);
        assert_eq!(state.borrow().release_calls, 1);
        assert_eq!(state.borrow().drop_calls, 1);
    }

    #[test]
    fn alternate_setting_mismatch_fails_closed_and_releases_the_claim() {
        let state = mock_state();
        state.borrow_mut().alternate_setting = 1;
        let result = CheckedBulkTransport::new(
            MockHandle(Rc::clone(&state)),
            mock_endpoints(),
            Duration::from_secs(1),
        );
        assert!(matches!(
            result,
            Err(UsbTransportError::AlternateSettingMismatch { actual: 1 })
        ));
        assert_eq!(state.borrow().release_calls, 1);
        assert_eq!(state.borrow().drop_calls, 1);
    }

    #[test]
    fn short_bulk_out_is_rejected_with_the_actual_byte_count() {
        let state = mock_state();
        state.borrow_mut().write_count = Some(2);
        let mut transport =
            CheckedBulkTransport::new(MockHandle(state), mock_endpoints(), Duration::from_secs(1))
                .unwrap();
        assert!(matches!(
            transport.bulk_out(&[1, 2, 3]),
            Err(UsbTransportError::ShortTransfer {
                direction: BulkDirection::Out,
                expected: 3,
                actual: 2
            })
        ));
    }

    #[test]
    fn short_bulk_in_returns_only_the_bytes_actually_received() {
        let state = mock_state();
        {
            let mut state = state.borrow_mut();
            state.read_data = vec![0xaa, 0xbb, 0xcc];
            state.read_count = Some(2);
        }
        let mut transport =
            CheckedBulkTransport::new(MockHandle(state), mock_endpoints(), Duration::from_secs(1))
                .unwrap();
        assert_eq!(transport.bulk_in(3).unwrap(), vec![0xaa, 0xbb]);
    }

    #[test]
    fn bulk_timeouts_are_reported_with_direction_and_configured_duration() {
        let state = mock_state();
        state.borrow_mut().write_timeout = true;
        let timeout = Duration::from_millis(250);
        let mut transport =
            CheckedBulkTransport::new(MockHandle(state), mock_endpoints(), timeout).unwrap();
        assert!(matches!(
            transport.bulk_out(&[0x42]),
            Err(UsbTransportError::TransferTimeout {
                direction: BulkDirection::Out,
                timeout: actual
            }) if actual == timeout
        ));
    }

    #[test]
    fn bulk_in_timeout_is_reported_as_a_timeout_not_an_empty_read() {
        let state = mock_state();
        state.borrow_mut().read_timeout = true;
        let timeout = Duration::from_secs(3);
        let mut transport =
            CheckedBulkTransport::new(MockHandle(state), mock_endpoints(), timeout).unwrap();
        assert!(matches!(
            transport.bulk_in(4),
            Err(UsbTransportError::TransferTimeout {
                direction: BulkDirection::In,
                timeout: actual
            }) if actual == timeout
        ));
    }

    #[test]
    fn non_target_active_configuration_is_not_changed_or_claimed() {
        let state = mock_state();
        state.borrow_mut().active_configuration = 2;
        let result = CheckedBulkTransport::new(
            MockHandle(Rc::clone(&state)),
            mock_endpoints(),
            Duration::from_secs(1),
        );
        assert!(matches!(
            result,
            Err(UsbTransportError::ActiveConfigurationMismatch { actual: 2 })
        ));
        assert_eq!(state.borrow().claim_calls, 0);
        assert_eq!(state.borrow().release_calls, 0);
        assert_eq!(state.borrow().drop_calls, 1);
    }

    #[test]
    fn sub_millisecond_timeout_is_rejected_before_claiming() {
        let state = mock_state();
        let result = CheckedBulkTransport::new(
            MockHandle(Rc::clone(&state)),
            mock_endpoints(),
            Duration::from_micros(500),
        );
        assert!(matches!(
            result,
            Err(UsbTransportError::InvalidTimeout { .. })
        ));
        assert_eq!(state.borrow().claim_calls, 0);
        assert_eq!(state.borrow().drop_calls, 1);
    }
}
