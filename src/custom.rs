use crate::device::{self, Control, ControlKind, Layout, UsbCandidate};
use eframe::egui;
use serde::{Deserialize, Serialize};

mod editor;
pub use editor::layout_dialog;

const MAX_BUTTONS: u8 = 12;
const MAX_KNOBS: usize = 79;
const FIRST_DIAL_SLOT: u8 = 13;
const LAST_DIAL_SLOT: u8 = FIRST_DIAL_SLOT + 3 * MAX_KNOBS as u8 - 1;
const BOARD_X: u16 = 160;
const BOARD_Y: u16 = 125;
const GRID: u16 = 5;
const KEY_SIZE: u8 = 22;
const MIN_SIZE: u8 = 15;
const MAX_SIZE: u8 = 40;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Position {
    /// The key code or the middle (push) slot of a dial.
    slot: u8,
    x: u16,
    y: u16,
    size: u8,
}

impl Position {
    fn knob_index(self) -> Option<usize> {
        let offset = self.slot.checked_sub(FIRST_DIAL_SLOT + 1)?;
        let index = usize::from(offset / 3);
        (offset.is_multiple_of(3) && index < MAX_KNOBS).then_some(index)
    }

    fn overlaps(self, other: Self) -> bool {
        let s = u16::from(self.size);
        let o = u16::from(other.size);
        self.x < other.x.saturating_add(o)
            && other.x < self.x.saturating_add(s)
            && self.y < other.y.saturating_add(o)
            && other.y < self.y.saturating_add(s)
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Preset {
    Keys3,
    Keys9,
    Keys12,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum Rotation {
    None,
    Left,
    Right,
    Both,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Knob {
    rotation: Rotation,
    push: bool,
}

impl Default for Knob {
    fn default() -> Self {
        Self {
            rotation: Rotation::Both,
            push: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomLayout {
    pub buttons: u8,
    knobs: Vec<Knob>,
    #[serde(skip_serializing_if = "Option::is_none")]
    positions: Option<Vec<Position>>,
}

impl Default for CustomLayout {
    fn default() -> Self {
        Self {
            buttons: 3,
            knobs: vec![Knob::default()],
            positions: None,
        }
    }
}

impl CustomLayout {
    pub fn from_layout(layout: &Layout) -> Self {
        let count = |kind| {
            layout
                .controls
                .iter()
                .filter(|control| control.kind == kind)
                .map(|control| control.id)
                .max()
                .unwrap_or(0)
        };
        let mut result = Self {
            buttons: count(ControlKind::Button).clamp(1, MAX_BUTTONS),
            knobs: vec![Knob::default(); usize::from(count(ControlKind::Knob)).min(MAX_KNOBS)],
            positions: Some(Vec::new()),
        };
        for control in &layout.controls {
            let slot = match control.kind {
                ControlKind::Button if (1..=MAX_BUTTONS).contains(&control.id) => control.id,
                ControlKind::Knob if (1..=MAX_KNOBS as u8).contains(&control.id) => {
                    14 + (control.id - 1) * 3
                }
                _ => continue,
            };
            if result.position(slot).is_some() {
                continue;
            }
            let size = control.width.min(control.height).round() as u8;
            let size = size.clamp(MIN_SIZE, MAX_SIZE);
            let position = Position {
                slot,
                x: aligned(control.x, BOARD_X, size),
                y: aligned(control.y, BOARD_Y, size),
                size,
            };
            let position = if result.can_place(position) {
                Some(position)
            } else {
                result.free_position(slot, size)
            };
            if let Some(position) = position {
                result.positions.as_mut().unwrap().push(position);
            }
        }
        result.sync_count();
        if result.is_valid() {
            result
        } else {
            Self::default()
        }
    }

    pub fn allows(&self, slot: u8) -> bool {
        match slot {
            1..=12 => self
                .positions
                .as_ref()
                .map_or(slot <= self.buttons, |_| self.position(slot).is_some()),
            FIRST_DIAL_SLOT..=LAST_DIAL_SLOT => {
                let offset = slot - FIRST_DIAL_SLOT;
                if self.positions.is_some() && self.position(14 + (offset / 3) * 3).is_none() {
                    return false;
                }
                self.knobs
                    .get(usize::from(offset / 3))
                    .is_some_and(|knob| match offset % 3 {
                        0 => matches!(knob.rotation, Rotation::Left | Rotation::Both),
                        1 => knob.push,
                        _ => matches!(knob.rotation, Rotation::Right | Rotation::Both),
                    })
            }
            _ => false,
        }
    }

    pub fn is_valid(&self) -> bool {
        if !(1..=MAX_BUTTONS).contains(&self.buttons) || self.knobs.len() > MAX_KNOBS {
            return false;
        }
        let Some(positions) = &self.positions else {
            return self.knobs.len() <= 3
                && self
                    .knobs
                    .iter()
                    .all(|knob| knob.push || knob.rotation != Rotation::None);
        };
        positions
            .iter()
            .filter(|item| item.slot <= MAX_BUTTONS)
            .count()
            == usize::from(self.buttons)
            && positions.iter().enumerate().all(|(index, item)| {
                ((1..=MAX_BUTTONS).contains(&item.slot)
                    || item.knob_index().is_some_and(|i| {
                        self.knobs
                            .get(i)
                            .is_some_and(|knob| knob.push || knob.rotation != Rotation::None)
                    }))
                    && self.can_place(*item)
                    && !positions[..index]
                        .iter()
                        .any(|other| other.slot == item.slot)
            })
    }

    fn preset(preset: Preset) -> Self {
        let (buttons, columns, knob_count, first_row) = match preset {
            Preset::Keys3 => (3, 3, 1, 5),
            Preset::Keys9 => (9, 3, 2, 35),
            Preset::Keys12 => (12, 4, 2, 35),
        };
        let mut positions = (1..=buttons)
            .map(|slot| Position {
                slot,
                x: 5 + u16::from((slot - 1) % columns) * 25,
                y: first_row + u16::from((slot - 1) / columns) * 25,
                size: KEY_SIZE,
            })
            .collect::<Vec<_>>();
        let far_edge = 5 + u16::from(columns - 1) * 25;
        positions.push(Position {
            slot: 14,
            x: if knob_count == 1 { far_edge + 30 } else { 5 },
            y: 5,
            size: KEY_SIZE,
        });
        if knob_count > 1 {
            positions.push(Position {
                slot: 17,
                x: far_edge,
                y: 5,
                size: KEY_SIZE,
            });
        }
        Self {
            buttons,
            knobs: vec![Knob::default(); knob_count],
            positions: Some(positions),
        }
    }

    fn position(&self, slot: u8) -> Option<Position> {
        self.positions
            .as_deref()?
            .iter()
            .find(|item| item.slot == slot)
            .copied()
    }

    fn edit_positions(&mut self) {
        if self.positions.is_none() {
            let mut positions = Vec::new();
            let columns = self.buttons.clamp(1, 4);
            positions.extend((1..=self.buttons).map(|slot| Position {
                slot,
                x: u16::from((slot - 1) % columns) * 25,
                y: u16::from((slot - 1) / columns) * 25,
                size: KEY_SIZE,
            }));
            positions.extend((0..self.knobs.len()).map(|i| Position {
                slot: 14 + 3 * i as u8,
                x: u16::from(columns) * 25 + 5,
                y: i as u16 * 25,
                size: KEY_SIZE,
            }));
            self.positions = Some(positions);
        }
    }

    fn can_place(&self, item: Position) -> bool {
        let size = u16::from(item.size);
        (MIN_SIZE..=MAX_SIZE).contains(&item.size)
            && item.x.saturating_add(size) <= BOARD_X
            && item.y.saturating_add(size) <= BOARD_Y
            && self
                .positions
                .as_deref()
                .unwrap_or_default()
                .iter()
                .all(|other| other.slot == item.slot || !item.overlaps(*other))
    }

    fn free_position(&self, slot: u8, size: u8) -> Option<Position> {
        let end_x = BOARD_X - u16::from(size);
        let end_y = BOARD_Y - u16::from(size);
        (0..=end_y).step_by(GRID.into()).find_map(|y| {
            (0..=end_x)
                .step_by(GRID.into())
                .map(|x| Position { slot, x, y, size })
                .find(|item| self.can_place(*item))
        })
    }

    fn replace(&mut self, item: Position) -> bool {
        if !self.can_place(item) {
            return false;
        }
        let Some(position) = self.positions.as_mut().and_then(|positions| {
            positions
                .iter_mut()
                .find(|position| position.slot == item.slot)
        }) else {
            return false;
        };
        *position = item;
        true
    }

    fn move_control(&mut self, slot: u8, x: f32, y: f32) -> bool {
        let Some(mut item) = self.position(slot) else {
            return false;
        };
        item.x = aligned(x, BOARD_X, item.size);
        item.y = aligned(y, BOARD_Y, item.size);
        self.replace(item)
    }

    fn sync_count(&mut self) {
        self.buttons = self
            .positions
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|item| item.slot <= MAX_BUTTONS)
            .count() as u8;
    }

    fn plan_add(&self, knob: bool) -> Option<Position> {
        let slot = if knob {
            (FIRST_DIAL_SLOT + 1..LAST_DIAL_SLOT)
                .step_by(3)
                .find(|slot| self.position(*slot).is_none())
        } else {
            (1..=MAX_BUTTONS).find(|slot| self.position(*slot).is_none())
        }?;
        self.free_position(slot, KEY_SIZE)
            .or_else(|| self.free_position(slot, MIN_SIZE))
    }

    fn add(&mut self, knob: bool) -> Option<u8> {
        let item = self.plan_add(knob)?;
        if let Some(index) = item.knob_index() {
            if self.knobs.len() <= index {
                self.knobs.resize(index + 1, Knob::default());
            }
            self.knobs[index] = Knob::default();
        }
        self.positions.as_mut()?.push(item);
        self.sync_count();
        Some(item.slot)
    }

    fn remove(&mut self, slot: u8) -> bool {
        if slot <= MAX_BUTTONS && self.buttons <= 1 {
            return false;
        }
        let Some(items) = self.positions.as_mut() else {
            return false;
        };
        let Some(index) = items.iter().position(|item| item.slot == slot) else {
            return false;
        };
        items.remove(index);
        self.sync_count();
        true
    }

    /// Changes drawing geometry while retaining the supplied device's capabilities.
    pub fn layout(&self, source: &Layout) -> Layout {
        if let Some(positions) = &self.positions {
            let controls = positions
                .iter()
                .map(|item| {
                    let (kind, id) = match item.knob_index() {
                        Some(index) => (ControlKind::Knob, index as u8 + 1),
                        None => (ControlKind::Button, item.slot),
                    };
                    Control {
                        kind,
                        id,
                        x: f32::from(item.x),
                        y: f32::from(item.y),
                        width: f32::from(item.size),
                        height: f32::from(item.size),
                    }
                })
                .collect();
            return Layout {
                name: "Layout personalizado".into(),
                controls,
                ..source.clone()
            };
        }
        let buttons = self.buttons.clamp(1, MAX_BUTTONS);
        let columns = buttons.min(4);
        let mut controls = (1..=buttons)
            .map(|id| Control {
                kind: ControlKind::Button,
                id,
                x: f32::from((id - 1) % columns) * 25.0,
                y: f32::from((id - 1) / columns) * 25.0,
                width: 22.0,
                height: 22.0,
            })
            .collect::<Vec<_>>();
        controls.extend(
            (1..=self.knobs.len().min(MAX_KNOBS) as u8).map(|id| Control {
                kind: ControlKind::Knob,
                id,
                x: f32::from(columns) * 25.0 + 5.0,
                y: f32::from(id - 1) * 25.0,
                width: 35.0,
                height: 22.0,
            }),
        );
        Layout {
            name: "Layout personalizado".into(),
            controls,
            ..source.clone()
        }
    }
}

fn aligned(value: f32, extent: u16, size: u8) -> u16 {
    let highest = (extent - u16::from(size)) / GRID;
    let value = (value / f32::from(GRID)).round() as u16;
    value.min(highest) * GRID
}

pub fn usb_dialog(ctx: &egui::Context, dialog: &mut Option<Result<Vec<UsbCandidate>, String>>) {
    let Some(results) = dialog.as_mut() else {
        return;
    };
    let mut visible = true;
    let mut close = false;
    egui::Window::new(crate::i18n::tr("Identificar USB"))
        .open(&mut visible)
        .collapsible(false)
        .default_width(460.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label(crate::i18n::tr("A busca lista interfaces HID USB e indica quais têm perfil de configuração."));
            ui.horizontal(|ui| {
                if ui.button(crate::i18n::tr("Buscar novamente")).clicked() {
                    *results = device::usb_candidates();
                }
                close = ui.button(crate::i18n::tr("Fechar")).clicked();
            });
            ui.separator();
            match results {
                Ok(devices) if devices.is_empty() => {
                    ui.label(crate::i18n::tr("Nenhuma interface HID USB de fabricante foi encontrada."));
                }
                Ok(devices) => {
                    egui::ScrollArea::vertical()
                        .max_height(370.0)
                        .show(ui, |ui| {
                            for device in devices {
                                ui.strong(crate::i18n::model(&device.model));
                                let details = format!(
                                    "{:04X}:{:04X} · {} · HID {:04X}:{:04X}",
                                    device.vid,
                                    device.pid,
                                    device.interface,
                                    device.usage_page,
                                    device.usage,
                                );
                                ui.monospace(&details);
                                if device.supported {
                                    ui.label(crate::i18n::tr("Perfil de configuração reconhecido."));
                                } else {
                                    ui.label(crate::i18n::tr("Protocolo de gravação ainda desconhecido."));
                                }
                                if let Some(layout) = &device.likely_layout {
                                    ui.label(if device.supported {
                                        format!("{} {layout}", crate::i18n::tr("Layout"))
                                    } else {
                                        format!("{} {layout}", crate::i18n::tr("Possível layout"))
                                    });
                                }
                                match (device.vid, device.pid) {
                                    (0x1189, 0x8840) => {
                                        ui.label(crate::i18n::tr("Há uma variante 8840 de 6 teclas com giratório e protocolo 03 FD na interface MI_00. A gravação nessa variante precisa de teste separado."));
                                    }
                                    (0x6d7b, 0xdcfa) => {
                                        ui.label(crate::i18n::tr("Família WebHub. A leitura de atalhos e os comandos RGB usam um protocolo próprio."));
                                    }
                                    _ => {}
                                }
                                if ui.small_button(crate::i18n::tr("Copiar identificação")).clicked() {
                                    ctx.copy_text(format!("{} · {details}", device.model));
                                }
                                ui.separator();
                            }
                        });
                }
                Err(error) => {
                    ui.label(crate::i18n::status(error));
                }
            }
        });
    if !visible || close {
        *dialog = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_dials_have_stable_visual_slots_and_stop_at_the_grid_edges() {
        let mut custom = CustomLayout::preset(Preset::Keys3);
        assert_eq!(custom.add(true), Some(17));
        assert_eq!(custom.add(true), Some(20));
        assert_eq!(custom.add(true), Some(23));
        assert!(custom.allows(22) && custom.allows(23) && custom.allows(24));
        assert!(custom.remove(17) && !custom.allows(17) && custom.allows(23));
        assert_eq!(custom.add(true), Some(17));
        for _ in 0..MAX_KNOBS {
            if custom.add(true).is_none() {
                break;
            }
        }
        assert!(custom.is_valid());
        assert!(custom.add(true).is_none());
        let stock = device::default_layout();
        let image = custom.layout(&stock);
        assert!(image
            .controls
            .iter()
            .any(|control| control.slots() == [22, 23, 24]));
        assert!(!stock
            .controls
            .iter()
            .any(|control| control.slots().contains(&23)));
    }

    #[test]
    fn larger_presets_use_square_keys_with_both_dials_above_them() {
        for preset in [Preset::Keys9, Preset::Keys12] {
            let layout = CustomLayout::preset(preset);
            let placements = layout.positions.as_deref().unwrap();
            let keys: Vec<_> = placements.iter().filter(|item| item.slot < 13).collect();
            let first_key = keys.iter().map(|item| item.y).min().unwrap();
            assert_eq!(keys.len(), if preset == Preset::Keys9 { 9 } else { 12 });
            assert!(keys.iter().all(|item| item.size == KEY_SIZE));
            assert!(placements
                .iter()
                .filter(|item| item.slot > 12)
                .all(|item| item.y + u16::from(item.size) < first_key));
            assert!(layout.is_valid());
        }
    }

    #[test]
    fn dragging_and_removing_keys_preserve_other_hid_slots_and_legacy_profiles() {
        let mut layout: CustomLayout =
            serde_json::from_str(r#"{"buttons":3,"knobs":[{"rotation":"Both","push":true}]}"#)
                .unwrap();
        assert!(layout.positions.is_none() && layout.allows(3) && layout.allows(14));
        layout.edit_positions();
        assert!(layout.move_control(3, 78.0, 52.0));
        assert_eq!(layout.position(3).map(|p| (p.x, p.y)), Some((80, 50)));
        assert!(layout.remove(2) && !layout.allows(2) && layout.allows(3));
        let restored: CustomLayout =
            serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
        assert_eq!(restored, layout);
        assert!(restored.allows(13) && restored.allows(15));
    }

    #[test]
    fn independent_knob_directions_map_to_their_protocol_slots() {
        let custom = CustomLayout {
            buttons: 2,
            positions: None,
            knobs: vec![
                Knob {
                    rotation: Rotation::Right,
                    push: false,
                },
                Knob {
                    rotation: Rotation::Left,
                    push: true,
                },
                Knob {
                    rotation: Rotation::None,
                    push: true,
                },
            ],
        };
        let slots = (0..=22)
            .filter(|slot| custom.allows(*slot))
            .collect::<Vec<_>>();
        assert_eq!(slots, [1, 2, 15, 16, 17, 20]);
        assert!(custom.is_valid());
    }

    #[test]
    fn custom_visual_layout_keeps_original_hardware_capabilities() {
        let source = Layout {
            name: "Dispositivo de teste".into(),
            layers: 3,
            max_sequence: 9,
            supports_delay: true,
            supports_led_color: false,
            led_modes: 4,
            controls: Vec::new(),
        };
        let custom = CustomLayout {
            buttons: 12,
            knobs: Vec::new(),
            positions: None,
        };
        let result = custom.layout(&source);
        assert_eq!(
            (result.layers, result.max_sequence, result.led_modes),
            (3, 9, 4)
        );
        assert!(result.supports_delay && !result.supports_led_color);
        assert_eq!(result.controls.len(), 12);
        assert_eq!(result.controls.last().unwrap().slots(), [12]);
        let template = CustomLayout::from_layout(&result);
        assert!(template.is_valid());
        assert_eq!(template.buttons, custom.buttons);
        assert!(template.allows(12) && !template.allows(14));
    }
}
