use super::{
    CustomLayout, Position, Preset, Rotation, BOARD_X, BOARD_Y, MAX_BUTTONS, MAX_SIZE, MIN_SIZE,
};
use crate::ui::board_art::{self, Dial, Motion};
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

const BOARD: Color32 = Color32::from_rgb(13, 24, 39);
const GRID_COLOR: Color32 = Color32::from_rgb(30, 46, 65);

#[derive(Clone, Copy, Default)]
struct EditorState {
    selected: Option<u8>,
    drag_offset: Option<(u8, Vec2)>,
}

pub fn layout_dialog(
    ctx: &egui::Context,
    dialog: &mut Option<CustomLayout>,
    pressed: u32,
    angles: [f32; 3],
) -> Option<CustomLayout> {
    let draft = dialog.as_mut()?;
    draft.edit_positions();
    let id = egui::Id::new("custom-visual-editor-state");
    let mut state = ctx
        .data(|data| data.get_temp::<EditorState>(id))
        .unwrap_or_default();
    if state
        .selected
        .and_then(|slot| draft.position(slot))
        .is_none()
    {
        state.selected = first_slot(draft);
    }

    let mut visible = true;
    let mut apply = false;
    let mut cancel = false;
    let width = (ctx.screen_rect().width() - 48.0).clamp(280.0, 570.0);
    let height = (ctx.screen_rect().height() - 149.0).max(190.0);
    egui::Window::new(crate::i18n::tr("Editor de layout"))
        .open(&mut visible)
        .collapsible(false)
        .resizable(false)
        .default_width(width)
        .max_width(width)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(height)
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.label(crate::i18n::tr(
                        "Arraste os controles. A grade mantém as posições alinhadas.",
                    ));
                    ui.horizontal_wrapped(|ui| {
                        for (title, preset) in [
                            (crate::i18n::tr("3 teclas + 1 giratório"), Preset::Keys3),
                            (crate::i18n::tr("9 teclas + 2 giratórios"), Preset::Keys9),
                            (crate::i18n::tr("12 teclas + 2 giratórios"), Preset::Keys12),
                        ] {
                            if ui.button(title).clicked() {
                                *draft = CustomLayout::preset(preset);
                                state = EditorState {
                                    selected: first_slot(draft),
                                    drag_offset: None,
                                };
                            }
                        }
                    });

                    canvas(ui, draft, &mut state, pressed, angles);
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(
                                draft.buttons < MAX_BUTTONS,
                                egui::Button::new(crate::i18n::tr("+ Tecla")),
                            )
                            .clicked()
                        {
                            state.selected = draft.add(false).or(state.selected);
                        }
                        if ui
                            .add_enabled(
                                draft.plan_add(true).is_some(),
                                egui::Button::new(crate::i18n::tr("+ Giratório")),
                            )
                            .on_disabled_hover_text(crate::i18n::tr(
                                "Espaço insuficiente para esse tamanho.",
                            ))
                            .clicked()
                        {
                            state.selected = draft.add(true).or(state.selected);
                        }
                        let removable = state
                            .selected
                            .is_some_and(|slot| slot > MAX_BUTTONS || draft.buttons > 1);
                        if ui
                            .add_enabled(
                                removable,
                                egui::Button::new(crate::i18n::tr("Remover selecionado")),
                            )
                            .clicked()
                        {
                            if let Some(slot) = state.selected {
                                draft.remove(slot);
                                state.selected = first_slot(draft);
                                state.drag_offset = None;
                            }
                        }
                    });
                    if let Some(position) = state.selected.and_then(|slot| draft.position(slot)) {
                        setup_control(ui, draft, position);
                    }
                });
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                apply = ui
                    .add_enabled(
                        draft.is_valid(),
                        egui::Button::new(crate::i18n::tr("Salvar layout")),
                    )
                    .clicked();
                cancel = ui.button(crate::i18n::tr("Cancelar")).clicked();
            });
        });

    if apply || !visible || cancel {
        ctx.data_mut(|data| data.remove::<EditorState>(id));
        return if apply {
            dialog.take()
        } else {
            *dialog = None;
            None
        };
    }
    ctx.data_mut(|data| data.insert_temp(id, state));
    None
}

fn first_slot(draft: &CustomLayout) -> Option<u8> {
    draft.positions.as_deref()?.first().map(|item| item.slot)
}

fn canvas(
    ui: &mut egui::Ui,
    draft: &mut CustomLayout,
    state: &mut EditorState,
    pressed: u32,
    angles: [f32; 3],
) {
    let width = ui.available_width().max(160.0);
    let scale = ((width - 20.0) / f32::from(BOARD_X)).min(2.05);
    let board_size = Vec2::new(f32::from(BOARD_X), f32::from(BOARD_Y)) * scale;
    let (frame, _) =
        ui.allocate_exact_size(Vec2::new(width, board_size.y + 20.0), egui::Sense::hover());
    let board = Rect::from_center_size(frame.center(), board_size);
    let painter = ui.painter_at(frame);
    painter.rect_filled(frame, 11, BOARD);
    for x in (0..=BOARD_X).step_by(10) {
        let x = board.left() + f32::from(x) * scale;
        painter.vline(x, board.y_range(), egui::Stroke::new(1.0_f32, GRID_COLOR));
    }
    for y in (0..=BOARD_Y).step_by(10) {
        let y = board.top() + f32::from(y) * scale;
        painter.hline(board.x_range(), y, egui::Stroke::new(1.0_f32, GRID_COLOR));
    }

    let physically_pressed = |slot| pressed.checked_shr(u32::from(slot)).unwrap_or(0) & 1 != 0;
    for item in draft.positions.as_deref().unwrap_or_default().to_vec() {
        let id = ui.make_persistent_id(("placed-control", item.slot));
        let rect = rect_at(board.min, scale, item);
        let response = ui.interact(rect, id, egui::Sense::click_and_drag());
        let name = control_name(item);
        if response.clicked() || response.drag_started() {
            state.selected = Some(item.slot);
        }
        if response.drag_started() {
            let press = ui
                .input(|input| input.pointer.press_origin())
                .unwrap_or(rect.center());
            state.drag_offset = Some((item.slot, press - rect.min));
        }
        if response.dragged() {
            if let (Some(pointer), Some((_, offset))) = (
                response.interact_pointer_pos(),
                state.drag_offset.filter(|(slot, _)| *slot == item.slot),
            ) {
                let position = (pointer - offset - board.min) / scale;
                draft.move_control(item.slot, position.x, position.y);
            }
        }
        if response.drag_stopped() {
            state.drag_offset = None;
        }
        let selected = state.selected == Some(item.slot);
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &name)
        });
        let hovered = response.hovered();
        let focused = response.has_focus() || selected;
        let down = response.is_pointer_button_down_on();
        let grabbing = response.dragged();
        response
            .on_hover_cursor(if grabbing {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Grab
            })
            .on_hover_text(&name);
        let item = draft.position(item.slot).unwrap_or(item);
        let rect = rect_at(board.min, scale, item);
        let motion = Motion::animate(
            ui.ctx(),
            id,
            hovered,
            down || physically_pressed(item.slot),
            selected,
            focused,
        );
        if let Some(number) = item.knob_index() {
            let dial = Dial::new(rect);
            let [left, _, right] = dial.targets();
            let pointer = ui
                .input(|input| input.pointer.hover_pos())
                .filter(|_| hovered && !grabbing);
            let mut turn = 0.0;
            for (index, slot, side) in [(0, item.slot - 1, left), (1, item.slot + 1, right)] {
                if draft.allows(slot) {
                    let edge = Motion::animate(
                        ui.ctx(),
                        id.with(index),
                        pointer.is_some_and(|point| side.contains(point)),
                        physically_pressed(slot),
                        false,
                        false,
                    );
                    dial.arrow(&painter, index == 0, edge);
                    let amount = edge.hover;
                    turn += if index == 0 { -amount } else { amount };
                }
            }
            let angle = angles.get(number).copied().unwrap_or_default() + turn * 0.23;
            dial.body(&painter, number as u8 + 1, motion, angle);
        } else {
            board_art::key(&painter, rect, item.slot, motion);
        }
    }
}

fn rect_at(origin: Pos2, scale: f32, placement: Position) -> Rect {
    Rect::from_min_size(
        origin + Vec2::new(f32::from(placement.x), f32::from(placement.y)) * scale,
        Vec2::splat(f32::from(placement.size) * scale),
    )
}

fn control_name(item: Position) -> String {
    match item.knob_index() {
        Some(i) => format!("{} {}", crate::i18n::tr("Giratório"), i + 1),
        None => format!("{} {}", crate::i18n::tr("Tecla"), item.slot),
    }
}

fn setup_control(ui: &mut egui::Ui, draft: &mut CustomLayout, mut item: Position) {
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        ui.strong(control_name(item));
        ui.label(crate::i18n::tr("Tamanho"));
        if ui
            .add(egui::DragValue::new(&mut item.size).range(MIN_SIZE..=MAX_SIZE))
            .changed()
            && !draft.replace(item)
        {
            ui.label(crate::i18n::tr("Espaço insuficiente para esse tamanho."));
        }
    });
    if let Some(index) = item.knob_index() {
        let knob = &mut draft.knobs[index];
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(
                &mut knob.rotation,
                Rotation::Both,
                crate::i18n::tr("Esquerda e direita"),
            );
            ui.selectable_value(
                &mut knob.rotation,
                Rotation::Left,
                crate::i18n::tr("Só esquerda"),
            );
            ui.selectable_value(
                &mut knob.rotation,
                Rotation::Right,
                crate::i18n::tr("Só direita"),
            );
            ui.selectable_value(
                &mut knob.rotation,
                Rotation::None,
                crate::i18n::tr("Sem giro"),
            );
            ui.checkbox(&mut knob.push, crate::i18n::tr("Apertar"));
        });
        if knob.rotation == Rotation::None && !knob.push {
            ui.label(crate::i18n::tr(
                "Selecione ao menos uma ação para este giratório.",
            ));
        }
    }
}
