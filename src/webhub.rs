//! WebHub packets adapted from ysalitrynskyi/MacroPad 5e6f8f03, GPL-3.0.
//! The fork credits dozzenn/knurl for WebHub protocol notes.

use super::{packet, Binding, REPORT_LEN};
use hidapi::HidDevice;
use std::time::{Duration, Instant};

pub(super) const READ_INFO: u8 = 5;
const READ_KEYS: u8 = 8;
const READ_LIGHT: u8 = 10;
pub(super) const WRITE_LIGHT: u8 = 11;
pub(super) const WRITE_KEY: u8 = 16;
const TABLE_LEN: usize = (16 + 3 * 3) * 4;
const BLOCK: usize = 56;
const PAYLOAD: usize = REPORT_LEN - 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RgbLighting {
    pub mode: u8,
    pub palette: bool,
    pub hue: u8,
    pub saturation: u8,
    /// HSV value is the visible illumination brightness on this family.
    pub value: u8,
    pub speed: u8,
}

impl Default for RgbLighting {
    fn default() -> Self {
        Self {
            mode: 0,
            palette: false,
            hue: 0,
            saturation: 255,
            value: 255,
            speed: 2,
        }
    }
}

fn frame(sub: u8, data: &[u8]) -> [u8; REPORT_LEN] {
    let mut buffer = packet(0, &[6, sub]);
    buffer[3..3 + data.len()].copy_from_slice(data);
    buffer
}

pub(super) fn info_request() -> [u8; REPORT_LEN] {
    frame(READ_INFO, &[])
}

pub(super) fn light_packet(light: RgbLighting) -> Result<[u8; REPORT_LEN], String> {
    if light.mode > 4 || light.speed > 4 {
        return Err("Modo RGB ou velocidade fora do intervalo do WebHub.".into());
    }
    let level = (u16::from(light.value) * 4 + 127) / 255;
    let uses_color = u8::from(!light.palette && light.mode != 0);
    Ok(frame(
        WRITE_LIGHT,
        &[
            11,
            0,
            0,
            1,
            0,
            light.mode,
            level as u8,
            light.speed,
            0,
            uses_color,
            0,
            light.hue,
            light.saturation,
            light.value,
        ],
    ))
}

fn parse_light(reply: &[u8]) -> Result<RgbLighting, String> {
    if reply.len() < 16 {
        return Err("Resposta RGB incompleta.".into());
    }
    if reply[7] > 4 || reply[9] > 4 || reply[11] > 1 {
        return Err("Estado RGB não reconhecido neste WebHub.".into());
    }
    Ok(RgbLighting {
        mode: reply[7],
        speed: reply[9],
        palette: reply[11] == 0,
        hue: reply[13],
        saturation: reply[14],
        value: reply[15],
    })
}

fn key_index(slot: u8) -> Option<usize> {
    match slot {
        1..=12 => Some(usize::from(slot - 1)),
        13..=21 => {
            let knob_slot = usize::from(slot - 13);
            let part = [2, 0, 1][knob_slot % 3];
            Some(16 + (knob_slot / 3) * 3 + part)
        }
        _ => None,
    }
}

pub(super) fn key_packet(
    slot: u8,
    layer: u8,
    binding: &Binding,
) -> Result<[u8; REPORT_LEN], String> {
    let index = key_index(slot).ok_or("Controle inválido para WebHub.")?;
    let bytes = match binding {
        Binding::Keyboard(keys) => match keys.as_slice() {
            [] | [(0, 0)] => [0x13, 0, 0, 0],
            [(modifiers, usage)] => [0x20, *modifiers, *usage, 0],
            _ => return Err("WebHub aceita uma tecla ou atalho por controle no aparelho.".into()),
        },
        Binding::Media(usage) => {
            let [first, second] = usage.to_le_bytes();
            [0x30, first, second, 0]
        }
        Binding::Mouse {
            buttons,
            scroll,
            modifiers: 0,
        } if *buttons <= 31 => [0x10, *buttons, 0, *scroll as u8],
        Binding::Mouse { .. } => return Err("WebHub não aceita modificadores para mouse.".into()),
    };
    let [first, second] = (index as u16 * 4).to_le_bytes();
    Ok(frame(
        WRITE_KEY,
        &[
            7,
            first,
            second,
            0,
            layer - 1,
            0,
            bytes[0],
            bytes[1],
            bytes[2],
            bytes[3],
        ],
    ))
}

fn parse_binding(entry: &[u8]) -> Option<Option<Binding>> {
    let &[kind, a, b, c] = entry else { return None };
    match (kind, a, b, c) {
        (0 | 0x13, ..) | (0x20 | 0x30, 0, 0, _) | (0x10, 0, _, 0) => Some(None),
        (0x20, ..) => Some(Some(Binding::Keyboard(vec![(a, b)]))),
        (0x30, ..) => Some(Some(Binding::Media(u16::from_le_bytes([a, b])))),
        (0x10, ..) if a <= 31 => Some(Some(Binding::Mouse {
            buttons: a,
            scroll: c as i8,
            modifiers: 0,
        })),
        // Native LED shortcuts (0x1f) and unknown types have no equivalent in Binding.
        _ => None,
    }
}

fn matches_answer(request: &[u8; REPORT_LEN], reply: &[u8]) -> bool {
    let sub = request[2];
    let reply_sub = if sub == READ_KEYS { 7 } else { sub };
    reply.len() >= 5
        && reply[0] == 0xaa
        && reply[1] == reply_sub
        && (!matches!(sub, READ_KEYS | WRITE_KEY) || reply[3..5] == request[4..6])
}

pub(super) trait Transport {
    fn write_frame(&self, request: &[u8; REPORT_LEN]) -> Result<(), String>;
    fn read_payload(&self, reply: &mut [u8; PAYLOAD], timeout: i32) -> Result<usize, String>;
}

impl Transport for HidDevice {
    fn write_frame(&self, request: &[u8; REPORT_LEN]) -> Result<(), String> {
        // windows-native returns zero for an immediately completed WriteFile.
        // A matching answer is required before declaring success in either case.
        match self.write(request).map_err(|error| error.to_string())? {
            0 | REPORT_LEN => Ok(()),
            written => Err(format!(
                "Quadro WebHub incompleto. {written} bytes enviados."
            )),
        }
    }

    fn read_payload(&self, reply: &mut [u8; PAYLOAD], timeout: i32) -> Result<usize, String> {
        // hidapi windows-native removes the zero report identifier for unnumbered reports.
        self.read_timeout(reply, timeout)
            .map_err(|error| error.to_string())
    }
}

pub(super) fn transfer(
    handle: &impl Transport,
    request: &[u8; REPORT_LEN],
) -> Result<Vec<u8>, String> {
    let sub = request[2];
    if request[0] != 0
        || request[1] != 6
        || !matches!(
            sub,
            READ_INFO | READ_KEYS | READ_LIGHT | WRITE_LIGHT | WRITE_KEY
        )
    {
        return Err("Quadro fora dos comandos WebHub suportados.".into());
    }
    let mut reply = [0; PAYLOAD];
    // Consume previously queued frames; a final nonblocking poll also starts
    // an overlapped read before the write on the Windows-native HID backend.
    let mut drained = false;
    for _ in 0..256 {
        if handle.read_payload(&mut reply, 0)? == 0 {
            drained = true;
            break;
        }
    }
    if !drained {
        return Err("Fila WebHub ocupada. Feche outros configuradores e tente novamente.".into());
    }
    handle.write_frame(request)?;
    let deadline = Instant::now() + Duration::from_secs(1);
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        let timeout = i32::try_from(remaining.as_millis()).unwrap_or(1000).max(1);
        let read = handle.read_payload(&mut reply, timeout)?;
        if read == 0 {
            break;
        }
        if let Some(payload) = reply
            .get(..read)
            .filter(|payload| matches_answer(request, payload))
        {
            return Ok(payload.to_vec());
        }
    }
    Err(format!("WebHub não confirmou o comando {sub:02X}."))
}

pub(super) fn read_light(handle: &impl Transport) -> Result<RgbLighting, String> {
    parse_light(&transfer(handle, &frame(READ_LIGHT, &[]))?)
}

pub(super) fn apply_light(
    handle: &impl Transport,
    light: RgbLighting,
) -> Result<RgbLighting, String> {
    transfer(handle, &light_packet(light)?)?;
    let state = read_light(handle)?;
    let same_color = light.mode == 0
        || (state.palette == light.palette
            && state.value == light.value
            && (light.palette || (state.hue, state.saturation) == (light.hue, light.saturation)));
    if state.mode != light.mode || !same_color || (light.mode >= 2 && state.speed != light.speed) {
        return Err(
            "O WebHub confirmou a escrita, mas a leitura retornou parâmetros RGB diferentes."
                .into(),
        );
    }
    Ok(state)
}

pub(super) fn read_keys(
    handle: &impl Transport,
    layer: u8,
    slots: impl IntoIterator<Item = u8>,
) -> Result<Vec<(u8, Option<Binding>)>, String> {
    let mut table = Vec::with_capacity(TABLE_LEN);
    for offset in (0..TABLE_LEN).step_by(BLOCK) {
        let [first, second] = (offset as u16).to_le_bytes();
        let request = frame(READ_KEYS, &[58, first, second, 0, layer - 1]);
        let reply = transfer(handle, &request)?;
        let needed = (TABLE_LEN - offset).min(BLOCK);
        let block = reply
            .get(8..8 + needed)
            .ok_or("Bloco de atalhos WebHub incompleto.")?;
        table.extend_from_slice(block);
    }
    Ok(slots
        .into_iter()
        .filter_map(|slot| {
            let offset = key_index(slot)? * 4;
            parse_binding(&table[offset..offset + 4]).map(|binding| (slot, binding))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    type Exchange = ([u8; REPORT_LEN], Vec<Vec<u8>>);

    struct Mock {
        steps: RefCell<VecDeque<Exchange>>,
        answers: RefCell<VecDeque<Vec<u8>>>,
    }

    impl Mock {
        fn new(steps: Vec<Exchange>) -> Self {
            Self {
                steps: RefCell::new(steps.into()),
                answers: RefCell::default(),
            }
        }
    }

    impl Transport for Mock {
        fn write_frame(&self, request: &[u8; REPORT_LEN]) -> Result<(), String> {
            let (expected, replies) = self
                .steps
                .borrow_mut()
                .pop_front()
                .expect("unexpected transfer");
            assert_eq!(*request, expected);
            self.answers.borrow_mut().extend(replies);
            Ok(())
        }

        fn read_payload(&self, reply: &mut [u8; PAYLOAD], _: i32) -> Result<usize, String> {
            let Some(data) = self.answers.borrow_mut().pop_front() else {
                return Ok(0);
            };
            reply[..data.len()].copy_from_slice(&data);
            Ok(data.len())
        }
    }

    #[test]
    fn rgb_writes_hsv_and_verifies_matching_state() {
        let desired = RgbLighting {
            mode: 2,
            palette: false,
            hue: 170,
            saturation: 210,
            value: 128,
            speed: 3,
        };
        let write = packet(0, &[6, 11, 11, 0, 0, 1, 0, 2, 2, 3, 0, 1, 0, 170, 210, 128]);
        let state = vec![0xaa, 10, 11, 0, 0, 1, 0, 2, 2, 3, 0, 1, 0, 170, 210, 128];
        let mock = Mock::new(vec![
            (write, vec![state.clone(), vec![0xaa, 11, 0, 0, 0]]),
            (packet(0, &[6, 10]), vec![state]),
        ]);
        assert_eq!(apply_light(&mock, desired), Ok(desired));
        assert!(mock.steps.borrow().is_empty());
    }

    #[test]
    fn readback_requires_correct_response_offsets_and_preserves_unknown_entries() {
        let mut first = vec![0; 64];
        first[..5].copy_from_slice(&[0xaa, 7, 58, 0, 0]);
        first[8..12].copy_from_slice(&[0x20, 1, 4, 0]);
        first[12..16].copy_from_slice(&[0x1f, 3, 0, 0]);
        let mut second = vec![0; 64];
        second[..5].copy_from_slice(&[0xaa, 7, 58, 56, 0]);
        let position = 8 + (18 * 4 - 56);
        second[position..position + 4].copy_from_slice(&[0x10, 0, 0, 0xff]);
        let mock = Mock::new(vec![
            (
                packet(0, &[6, 8, 58, 0, 0, 0, 0]),
                vec![second.clone(), first],
            ),
            (packet(0, &[6, 8, 58, 56, 0, 0, 0]), vec![second]),
        ]);
        assert_eq!(
            read_keys(&mock, 1, [1, 2, 3, 13]).unwrap(),
            vec![
                (1, Some(Binding::Keyboard(vec![(1, 4)]))),
                (3, None),
                (
                    13,
                    Some(Binding::Mouse {
                        buttons: 0,
                        scroll: -1,
                        modifiers: 0
                    })
                ),
            ]
        );
    }

    #[test]
    fn knob_write_uses_webhub_slot_order_and_layer() {
        assert_eq!(
            key_packet(13, 2, &Binding::Media(0xe9)).unwrap(),
            packet(0, &[6, 16, 7, 72, 0, 0, 1, 0, 0x30, 0xe9, 0, 0])
        );
        assert_eq!(
            key_packet(14, 1, &Binding::Keyboard(vec![])).unwrap(),
            packet(0, &[6, 16, 7, 64, 0, 0, 0, 0, 0x13, 0, 0, 0])
        );
        assert!(key_packet(
            14,
            1,
            &Binding::Mouse {
                buttons: 1,
                scroll: 0,
                modifiers: 1
            }
        )
        .is_err());
    }

    #[test]
    fn unknown_commands_never_write() {
        let mock = Mock::new(Vec::new());
        assert!(transfer(&mock, &frame(0x5a, &[])).is_err());
    }

    #[test]
    fn light_write_ack_is_not_in_itself_success() {
        let desired = RgbLighting {
            mode: 1,
            ..RgbLighting::default()
        };
        let mock = Mock::new(vec![
            (
                light_packet(desired).unwrap(),
                vec![vec![0xaa, 11, 0, 0, 0]],
            ),
            (
                packet(0, &[6, 10]),
                vec![vec![
                    0xaa, 10, 11, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 255, 255,
                ]],
            ),
        ]);
        assert!(apply_light(&mock, desired).is_err());
    }
}
