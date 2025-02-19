use core::mem::MaybeUninit;

use super::cmd::{AclDataPacket, AclDataSerial};
use super::consts::TlPacketType;
use super::{PacketHeader, TL_EVT_HEADER_SIZE};
use crate::tl_mbox::mm;

/// the payload of [`Evt`] for a command status event
#[derive(Copy, Clone)]
#[repr(C, packed)]
pub struct CsEvt {
    pub status: u8,
    pub num_cmd: u8,
    pub cmd_code: u16,
}

/// the payload of [`Evt`] for a command complete event
#[derive(Clone, Copy, Default)]
#[repr(C, packed)]
pub struct CcEvt {
    pub num_cmd: u8,
    pub cmd_code: u8,
    pub payload: [u8; 1],
}

#[derive(Clone, Copy, Default)]
#[repr(C, packed)]
pub struct Evt {
    pub evt_code: u8,
    pub payload_len: u8,
    pub payload: [u8; 1],
}

#[derive(Clone, Copy, Default)]
#[repr(C, packed)]
pub struct EvtSerial {
    pub kind: u8,
    pub evt: Evt,
}

/// This format shall be used for all events (asynchronous and command response) reported
/// by the CPU2 except for the command response of a system command where the header is not there
/// and the format to be used shall be `EvtSerial`.
///
/// ### Note:
/// Be careful that the asynchronous events reported by the CPU2 on the system channel do
/// include the header and shall use `EvtPacket` format. Only the command response format on the
/// system channel is different.
#[derive(Clone, Copy, Default)]
#[repr(C, packed)]
pub struct EvtPacket {
    pub header: PacketHeader,
    pub evt_serial: EvtSerial,
}

/// Smart pointer to the [`EvtPacket`] that will dispose of it automatically on drop
pub struct EvtBox {
    ptr: *mut EvtPacket,
}

unsafe impl Send for EvtBox {}
impl EvtBox {
    pub(super) fn new(ptr: *mut EvtPacket) -> Self {
        Self { ptr }
    }

    /// Copies the event data from inner pointer and returns and event structure
    pub fn evt(&self) -> EvtPacket {
        let mut evt = MaybeUninit::uninit();
        unsafe {
            self.ptr.copy_to(evt.as_mut_ptr(), 1);
            evt.assume_init()
        }
    }

    /// Returns the size of a buffer required to hold this event
    pub fn size(&self) -> Result<usize, ()> {
        unsafe {
            let evt_kind = TlPacketType::try_from((*self.ptr).evt_serial.kind)?;

            if evt_kind == TlPacketType::AclData {
                let acl_data: *const AclDataPacket = self.ptr.cast();
                let acl_serial: *const AclDataSerial = &(*acl_data).acl_data_serial;

                Ok((*acl_serial).length as usize + 5)
            } else {
                let evt_data: *const EvtPacket = self.ptr.cast();
                let evt_serial: *const EvtSerial = &(*evt_data).evt_serial;

                Ok((*evt_serial).evt.payload_len as usize + TL_EVT_HEADER_SIZE)
            }
        }
    }

    /// writes an underlying [`EvtPacket`] into the provided buffer. Returns the number of bytes that were
    /// written. Returns an error if event kind is unkown or if provided buffer size is not enough
    pub fn copy_into_slice(&self, buf: &mut [u8]) -> Result<usize, ()> {
        unsafe {
            let evt_kind = TlPacketType::try_from((*self.ptr).evt_serial.kind)?;

            let evt_data: *const EvtPacket = self.ptr.cast();
            let evt_serial: *const EvtSerial = &(*evt_data).evt_serial;
            let evt_serial_buf: *const u8 = evt_serial.cast();

            let acl_data: *const AclDataPacket = self.ptr.cast();
            let acl_serial: *const AclDataSerial = &(*acl_data).acl_data_serial;
            let acl_serial_buf: *const u8 = acl_serial.cast();

            if let TlPacketType::AclData = evt_kind {
                let len = (*acl_serial).length as usize + 5;
                if len > buf.len() {
                    return Err(());
                }

                core::ptr::copy(evt_serial_buf, buf.as_mut_ptr(), len);

                Ok(len)
            } else {
                let len = (*evt_serial).evt.payload_len as usize + TL_EVT_HEADER_SIZE;
                if len > buf.len() {
                    return Err(());
                }

                core::ptr::copy(acl_serial_buf, buf.as_mut_ptr(), len);

                Ok(len)
            }
        }
    }
}

impl Drop for EvtBox {
    fn drop(&mut self) {
        use crate::ipcc::Ipcc;

        let mut ipcc = Ipcc::new_inner(unsafe { crate::Peripherals::steal() }.IPCC);
        mm::MemoryManager::evt_drop(self.ptr, &mut ipcc);
    }
}
