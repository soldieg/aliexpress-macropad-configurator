//! 1189:8840 MI_00 FD variant from coldmon91/MacroPad2 3a3e31e2, GPL-3.0.
//! Supported packets here are the keyboard and rotary wheel actually described as tested upstream.

use super::{packet, Binding, Device, REPORT_LEN};
#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use std::fs::{File, OpenOptions};
#[cfg(windows)]
use std::io;
#[cfg(windows)]
use std::os::windows::{
    fs::OpenOptionsExt,
    io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
};
#[cfg(windows)]
use std::ptr::{null, null_mut};
#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
const GENERIC_READ_WRITE: u32 = 0xc000_0000;
#[cfg(windows)]
const OVERLAPPED_IO: u32 = 0x4000_0000;
#[cfg(windows)]
const IO_PENDING: i32 = 997;

#[cfg(windows)]
#[repr(C)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: RawHandle,
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn CreateEventW(attrs: *const c_void, manual: i32, initial: i32, name: *const u16)
        -> RawHandle;
    fn WriteFile(
        handle: RawHandle,
        buffer: *const u8,
        len: u32,
        written: *mut u32,
        io: *mut Overlapped,
    ) -> i32;
    fn GetOverlappedResultEx(
        handle: RawHandle,
        io: *mut Overlapped,
        written: *mut u32,
        ms: u32,
        alertable: i32,
    ) -> i32;
    fn GetOverlappedResult(
        handle: RawHandle,
        io: *mut Overlapped,
        written: *mut u32,
        wait: i32,
    ) -> i32;
    fn CancelIoEx(handle: RawHandle, io: *mut Overlapped) -> i32;
}

#[cfg(windows)]
#[link(name = "hid")]
extern "system" {
    fn HidD_GetPreparsedData(handle: RawHandle, data: *mut *mut c_void) -> u8;
    fn HidD_FreePreparsedData(data: *mut c_void) -> u8;
    fn HidP_GetCaps(data: *const c_void, caps: *mut u16) -> i32;
}

#[cfg(windows)]
fn open(device: &Device) -> Result<File, String> {
    let path = device.path.to_str().map_err(|_| "Caminho HID inválido.")?;
    let handle = OpenOptions::new()
        .access_mode(GENERIC_READ_WRITE)
        .share_mode(0)
        .custom_flags(OVERLAPPED_IO)
        .open(path)
        .map_err(|error| format!("Falha ao abrir o canal FD exclusivo. {error}"))?;
    let mut preparsed = null_mut();
    if unsafe { HidD_GetPreparsedData(handle.as_raw_handle(), &mut preparsed) } == 0 {
        return Err("Não foi possível consultar o descritor FD.".into());
    }
    // HIDP_CAPS is 32 u16 entries. Its UsagePage and OutputReportByteLength
    // are at 1 and 3 respectively.
    let mut caps = [0u16; 32];
    let status = unsafe { HidP_GetCaps(preparsed, caps.as_mut_ptr()) };
    unsafe { HidD_FreePreparsedData(preparsed) };
    if status < 0 || (caps[1], usize::from(caps[3])) != (0xff00, REPORT_LEN) {
        return Err("Descritor da coleção FD incompatível.".into());
    }
    Ok(handle)
}

#[cfg(windows)]
pub(super) fn probe(device: &Device) -> Result<u8, String> {
    // The upstream FD probe uses the fixed descriptor ID without sending any write.
    let _handle = open(device)?;
    Ok(3)
}

#[cfg(not(windows))]
pub(super) fn probe(_: &Device) -> Result<u8, String> {
    Err("O transporte deste protocolo FD está disponível no Windows.".into())
}

pub(super) fn reports(
    slot: u8,
    layer: u8,
    binding: &Binding,
) -> Result<Vec<[u8; REPORT_LEN]>, String> {
    let native_slot = match slot {
        1..=6 => slot,
        13..=15 => slot + 3,
        _ => return Err("Controle inválido para o modelo FD.".into()),
    };
    let mut report = packet(3, &[0xfd, native_slot, layer]);
    match binding {
        Binding::Keyboard(keys) if !keys.is_empty() && keys.len() <= 27 => {
            report[4] = 1;
            report[10] = keys.len() as u8;
            for (index, &(mods, key)) in keys.iter().enumerate() {
                report[11 + index * 2..13 + index * 2].copy_from_slice(&[mods, key]);
            }
        }
        Binding::Mouse {
            buttons: 0,
            scroll,
            modifiers,
        } if matches!(scroll, -1 | 1) => {
            report[4] = 3;
            report[10] = 3;
            report[11] = *modifiers;
            report[15] = *scroll as u8;
        }
        _ => return Err("Neste FD, o formato de teclado e de rolagem foi documentado.".into()),
    }
    let separator = packet(3, &[0xaa, 0xaa]);
    Ok(vec![
        report,
        separator,
        packet(3, &[0xfd, 0xfe, 0xff]),
        separator,
    ])
}

#[cfg(windows)]
fn write(handle: &File, report: &[u8; REPORT_LEN]) -> io::Result<()> {
    let raw_event = unsafe { CreateEventW(null(), 1, 0, null()) };
    if raw_event.is_null() {
        return Err(io::Error::last_os_error());
    }
    let event = unsafe { OwnedHandle::from_raw_handle(raw_event) };
    let raw_handle = handle.as_raw_handle();
    let mut overlap = Overlapped {
        internal: 0,
        internal_high: 0,
        offset: u32::MAX,
        offset_high: u32::MAX,
        event: event.as_raw_handle(),
    };
    let sent = unsafe {
        WriteFile(
            raw_handle,
            report.as_ptr(),
            REPORT_LEN as u32,
            null_mut(),
            &mut overlap,
        )
    };
    if sent == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(IO_PENDING) {
            return Err(error);
        }
    }
    let mut written = 0;
    let completed =
        unsafe { GetOverlappedResultEx(raw_handle, &mut overlap, &mut written, 1000, 0) };
    if completed == 0 {
        let error = io::Error::last_os_error();
        // Request cancellation and drain the kernel's reference to the
        // overlapped structure and output buffer before returning.
        unsafe {
            CancelIoEx(raw_handle, &mut overlap);
            GetOverlappedResult(raw_handle, &mut overlap, &mut written, 1);
        }
        return Err(error);
    }
    if written != REPORT_LEN as u32 {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            format!("{written} bytes enviados"),
        ));
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn send(device: &Device, reports: &[[u8; REPORT_LEN]]) -> Result<(), String> {
    let (blocks, remainder) = reports.as_chunks::<4>();
    if !remainder.is_empty()
        || blocks.iter().any(|block| {
            block[0][0] != 3
                || block[0][1] != 0xfd
                || !matches!(block[0][2], 1..=6 | 16..=18)
                || block[0][3] != 1
                || !matches!(block[0][4], 1 | 3)
                || block[1] != packet(3, &[0xaa, 0xaa])
                || block[2] != packet(3, &[0xfd, 0xfe, 0xff])
                || block[3] != block[1]
        })
    {
        return Err("Sequência de gravação FD incompatível.".into());
    }
    let handle = open(device)?;
    for block in blocks {
        for report in block {
            write(&handle, report)
                .map_err(|error| format!("Falha durante a gravação FD. {error}"))?;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

#[cfg(not(windows))]
pub(super) fn send(_: &Device, _: &[[u8; REPORT_LEN]]) -> Result<(), String> {
    Err("O transporte deste protocolo FD está disponível no Windows.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Binding;

    #[test]
    fn documented_reference_frames_for_keyboard_and_rotary() {
        let keyboard = Binding::Keyboard(vec![
            (0, 56),
            (0, 16),
            (0, 18),
            (0, 7),
            (0, 8),
            (0, 15),
            (0, 44),
        ]);
        let reports = reports(1, 1, &keyboard).unwrap();
        assert_eq!(
            &reports[0][..25],
            &[3, 253, 1, 1, 1, 0, 0, 0, 0, 0, 7, 0, 56, 0, 16, 0, 18, 0, 7, 0, 8, 0, 15, 0, 44]
        );
        assert_eq!(&reports[1][..4], &[3, 170, 170, 0]);
        assert_eq!(&reports[2][..4], &[3, 253, 254, 255]);
        assert_eq!(&reports[3][..4], &[3, 170, 170, 0]);
        let rotary = Binding::Mouse {
            buttons: 0,
            scroll: -1,
            modifiers: 0,
        };
        assert_eq!(
            &super::reports(13, 1, &rotary).unwrap()[0][..16],
            &[3, 253, 16, 1, 3, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0, 255]
        );
    }
}
