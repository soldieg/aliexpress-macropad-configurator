//! Core Audio implementation for the current default output device.
use std::{ffi::c_void, mem::size_of, ptr::null};

const SYSTEM: u32 = 1;
const MAIN: u32 = 0;
const GLOBAL: u32 = u32::from_be_bytes(*b"glob");
const OUTPUT: u32 = u32::from_be_bytes(*b"outp");
const DEFAULT_OUTPUT: u32 = u32::from_be_bytes(*b"dOut");
const VIRTUAL_VOLUME: u32 = u32::from_be_bytes(*b"vmvc");
const VOLUME: u32 = u32::from_be_bytes(*b"volm");
const MUTE: u32 = u32::from_be_bytes(*b"mute");

#[repr(C)]
#[derive(Clone, Copy)]
struct Address {
    selector: u32,
    scope: u32,
    element: u32,
}

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectGetPropertyData(
        object: u32,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> i32;
    fn AudioObjectSetPropertyData(
        object: u32,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: u32,
        data: *const c_void,
    ) -> i32;
    fn AudioObjectIsPropertySettable(object: u32, address: *const Address, value: *mut u8) -> i32;
}

#[link(name = "AudioToolbox", kind = "framework")]
extern "C" {
    fn AudioHardwareServiceGetPropertyData(
        object: u32,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> i32;
    fn AudioHardwareServiceSetPropertyData(
        object: u32,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: u32,
        data: *const c_void,
    ) -> i32;
    fn AudioHardwareServiceIsPropertySettable(
        object: u32,
        address: *const Address,
        value: *mut u8,
    ) -> i32;
}

#[derive(Clone, Copy)]
struct Control {
    device: u32,
    address: Address,
}

impl Control {
    fn get<T: Default>(self) -> Result<T, String> {
        let read = if self.address.selector == VIRTUAL_VOLUME {
            AudioHardwareServiceGetPropertyData
        } else {
            AudioObjectGetPropertyData
        };
        let mut value = T::default();
        let expected = size_of::<T>() as u32;
        let mut size = expected;
        let status = unsafe {
            read(
                self.device,
                &self.address,
                0,
                null(),
                &mut size,
                (&mut value as *mut T).cast(),
            )
        };
        if status == 0 && size == expected {
            Ok(value)
        } else {
            Err(format!(
                "A propriedade do Core Audio está indisponível ({status})."
            ))
        }
    }

    fn settable(self) -> bool {
        let check = if self.address.selector == VIRTUAL_VOLUME {
            AudioHardwareServiceIsPropertySettable
        } else {
            AudioObjectIsPropertySettable
        };
        let mut value = 0;
        (unsafe { check(self.device, &self.address, &mut value) }) == 0 && value != 0
    }

    fn set<T>(self, value: &T) -> Result<(), String> {
        let write = if self.address.selector == VIRTUAL_VOLUME {
            AudioHardwareServiceSetPropertyData
        } else {
            AudioObjectSetPropertyData
        };
        let status = unsafe {
            write(
                self.device,
                &self.address,
                0,
                null(),
                size_of::<T>() as u32,
                (value as *const T).cast(),
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(format!(
                "O dispositivo de áudio recusou o ajuste ({status})."
            ))
        }
    }
}

fn output_device() -> Result<u32, String> {
    let device = Control {
        device: SYSTEM,
        address: Address {
            selector: DEFAULT_OUTPUT,
            scope: GLOBAL,
            element: MAIN,
        },
    }
    .get()?;
    (device != 0)
        .then_some(device)
        .ok_or_else(|| "Saída de áudio padrão indisponível.".into())
}

fn output_control(device: u32, selector: u32) -> Control {
    Control {
        device,
        address: Address {
            selector,
            scope: OUTPUT,
            element: MAIN,
        },
    }
}

fn valid_level(value: f32) -> Option<f32> {
    (value.is_finite() && (0.0..=1.0).contains(&value)).then_some(value)
}

fn volume(writable: bool) -> Result<(Control, f32), String> {
    let device = output_device()?;
    [VIRTUAL_VOLUME, VOLUME]
        .into_iter()
        .find_map(|selector| {
            let control = output_control(device, selector);
            if writable && !control.settable() {
                return None;
            }
            Some((control, valid_level(control.get().ok()?)?))
        })
        .ok_or_else(|| "A saída atual não oferece controle de volume principal no macOS.".into())
}

pub fn current() -> Result<u8, String> {
    let (_, value) = volume(false)?;
    Ok((value * 100.0).round() as u8)
}

pub fn adjust(step: i8) -> Result<u8, String> {
    let (control, value) = volume(true)?;
    let requested = (value + f32::from(step) / 100.0).clamp(0.0, 1.0);
    control.set(&requested)?;
    let actual = valid_level(control.get()?).ok_or("A saída retornou um volume inválido.")?;
    Ok((actual * 100.0).round() as u8)
}

pub fn toggle_mute() -> Result<(), String> {
    let control = output_control(output_device()?, MUTE);
    if !control.settable() {
        return Err("A saída atual não oferece controle de mudo no macOS.".into());
    }
    let muted: u32 = control.get()?;
    control.set(&u32::from(muted == 0))
}
