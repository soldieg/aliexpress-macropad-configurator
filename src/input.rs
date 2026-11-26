use crate::device::Binding;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Key { index: u8, down: bool, at: Instant },
    VolumeKey { index: u8, down: bool },
    Observed(DirectInput, String),
    Disconnected,
    ShowWindow,
    Quit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectInput {
    Keyboard {
        scan: u16,
        vk: u16,
        modifiers: u8,
    },
    Media(u16),
    Mouse {
        buttons: u8,
        delta: i16,
        modifiers: u8,
    },
}

impl DirectInput {
    pub fn function_key(vk: u16) -> Self {
        Self::Keyboard {
            scan: 0,
            vk,
            modifiers: 0,
        }
    }

    fn matches_key(self, modifiers: u8, usage: u8) -> bool {
        let Self::Keyboard {
            scan,
            vk,
            modifiers: observed,
        } = self
        else {
            return false;
        };
        observed == modifiers
            && match usage {
                0x48 => vk == 0x13,
                0x68..=0x73 => vk == 0x7c + u16::from(usage - 0x68),
                _ => scan_code(usage) == Some(scan),
            }
    }

    pub fn matches_binding(self, binding: &Binding) -> bool {
        match binding {
            Binding::Keyboard(keys) => keys
                .first()
                .is_some_and(|(modifiers, usage)| self.matches_key(*modifiers, *usage)),
            Binding::Media(usage) => self == Self::Media(*usage),
            Binding::Mouse {
                buttons,
                scroll,
                modifiers,
            } => {
                let Self::Mouse {
                    buttons: down,
                    delta,
                    modifiers: observed,
                } = self
                else {
                    return false;
                };
                *buttons == down
                    && *modifiers == observed
                    && (delta == 120 * i16::from(*scroll)
                        || (delta != 0
                            && delta.abs_diff(0) < 120
                            && i16::from(*scroll) == delta.signum()))
            }
        }
    }

    pub fn matches_later_step(self, binding: &Binding) -> bool {
        matches!(binding, Binding::Keyboard(keys) if keys.iter().skip(1).any(|(mods, key)| self.matches_key(*mods, *key)))
    }
}

pub fn volume_slot(index: u8) -> Option<u8> {
    (index < 6).then(|| 13 + (index / 2) * 3 + (index % 2) * 2)
}

pub fn volume_key(slot: u8) -> Option<u8> {
    (0..6)
        .find(|index| volume_slot(*index) == Some(slot))
        .map(|index| 108 + index)
}

pub(crate) fn scan_code(usage: u8) -> Option<u16> {
    // USB keyboard usage to PC/AT Set 1 make code. The high byte, when E0,
    // is passed separately to SendInput. Letter keys are physical positions.
    const LETTERS: [u16; 26] = [
        0x1e, 0x30, 0x2e, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, 0x31, 0x18,
        0x19, 0x10, 0x13, 0x1f, 0x14, 0x16, 0x2f, 0x11, 0x2d, 0x15, 0x2c,
    ];
    const STANDARD_AND_PAD: [u16; 62] = [
        0x1c, 0x01, 0x0e, 0x0f, 0x39, 0x0c, 0x0d, 0x1a, 0x1b, 0x2b, 0x2b, 0x27, 0x28, 0x29, 0x33,
        0x34, 0x35, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f, 0x40, 0x41, 0x42, 0x43, 0x44, 0x57, 0x58,
        0xe037, 0x46, 0, 0xe052, 0xe047, 0xe049, 0xe053, 0xe04f, 0xe051, 0xe04d, 0xe04b, 0xe050,
        0xe048, 0x45, 0xe035, 0x37, 0x4a, 0x4e, 0xe01c, 0x4f, 0x50, 0x51, 0x4b, 0x4c, 0x4d, 0x47,
        0x48, 0x49, 0x52, 0x53, 0x56, 0xe05d,
    ];
    let code = match usage {
        0x04..=0x1d => LETTERS[(usage - 4) as usize],
        0x1e..=0x27 => u16::from(usage - 0x1c),
        0x28..=0x65 => STANDARD_AND_PAD[(usage - 0x28) as usize],
        0x87 => 0x73,
        0x89 => 0x7d,
        0xe0..=0xe7 => [0x1d, 0x2a, 0x38, 0xe05b, 0xe01d, 0x36, 0xe038, 0xe05c][(usage - 0xe0) as usize],
        _ => return None,
    };
    (code != 0).then_some(code)
}

#[cfg(not(windows))]
impl DirectInput {
    pub(crate) fn keyboard_usage(usage: u8, modifiers: u8) -> Option<Self> {
        let (scan, vk) = match usage {
            0x48 => (0, 0x13),
            0x68..=0x73 => (0, 0x7c + u16::from(usage - 0x68)),
            _ => (scan_code(usage)?, 0),
        };
        Some(Self::Keyboard { scan, vk, modifiers })
    }
}
