use super::*;
use crate::i18n::{system_text, tr};
use egui::{Color32, FontId, Rect, RichText, Vec2};
pub(crate) mod board_art;

const BG: Color32 = Color32::from_rgb(13, 20, 32);
const PANEL: Color32 = Color32::from_rgb(22, 33, 51);
const EDGE: Color32 = Color32::from_rgb(45, 63, 88);
const BLUE: Color32 = Color32::from_rgb(121, 211, 249);
const MUTED: Color32 = Color32::from_rgb(167, 183, 203);
const CORAL: Color32 = Color32::from_rgb(255, 146, 142);
const DELETE_LAYOUT: &str = "confirm-delete-visual-layout";
const MEDIA: [(&str, u16); 6] = [
    ("Reproduzir / pausar", 0xcd),
    ("Próxima faixa", 0xb5),
    ("Faixa anterior", 0xb6),
    ("Silenciar", 0xe2),
    ("Aumentar volume", 0xe9),
    ("Diminuir volume", 0xea),
];
const MOUSE: [(&str, u8, i8); 5] = [
    ("Clique esquerdo", 1, 0),
    ("Clique direito", 2, 0),
    ("Clique central", 4, 0),
    ("Rolar para cima", 0, 1),
    ("Rolar para baixo", 0, -1),
];

pub(super) fn theme(ctx: &egui::Context) {
    let mut s = (*ctx.style()).clone();
    s.visuals = egui::Visuals::dark();
    s.visuals.panel_fill = BG;
    s.visuals.window_fill = PANEL;
    s.visuals.window_stroke = egui::Stroke::new(1.0_f32, EDGE);
    s.visuals.widgets.inactive.bg_fill = Color32::from_rgb(32, 52, 74);
    s.visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 71, 100);
    s.visuals.selection.bg_fill = Color32::from_rgb(35, 80, 115);
    s.visuals.selection.stroke = egui::Stroke::new(1.0_f32, BLUE);
    s.spacing.item_spacing = Vec2::new(10.0, 9.0);
    s.spacing.button_padding = Vec2::new(13.0, 7.0);
    s.text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(15.0));
    ctx.set_style(s);
}

fn hint(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(RichText::new(i18n::status(&text.into())).color(MUTED));
}

fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(egui::Stroke::new(1.0_f32, EDGE))
        .corner_radius(14)
        .inner_margin(18)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}

pub(super) fn slot_name(slot: u8) -> String {
    if slot < 13 {
        format!("{} {slot}", tr("Tecla"))
    } else {
        let id = (slot - 13) / 3 + 1;
        let action = ["Girar à esquerda", "Apertar", "Girar à direita"][((slot - 13) % 3) as usize];
        format!("{} · {} {id}", tr(action), tr("Giratório"))
    }
}

fn choices() -> &'static [(u8, String)] {
    static CHOICES: std::sync::OnceLock<Vec<(u8, String)>> = std::sync::OnceLock::new();
    CHOICES.get_or_init(|| {
        let mut out = (0..26)
            .map(|i| (4 + i, ((b'A' + i) as char).to_string()))
            .collect::<Vec<_>>();
        out.extend((0..10).map(|i| (30 + i, ((i + 1) % 10).to_string())));
        out.extend((1..=24).map(|i| (if i <= 12 { 57 + i } else { 91 + i }, format!("F{i}"))));
        out.extend(
            [
                (40, "Enter"),
                (41, "Escape"),
                (42, "Backspace"),
                (43, "Tab"),
                (44, "Space"),
                (45, "Minus"),
                (46, "Equals"),
                (47, "OpenBracket"),
                (48, "CloseBracket"),
                (49, "Backslash"),
                (51, "Semicolon"),
                (52, "Quote"),
                (54, "Comma"),
                (55, "Period"),
                (56, "Slash"),
                (57, "Caps Lock"),
                (70, "Print Screen"),
                (71, "Scroll Lock"),
                (72, "Pause"),
                (73, "Insert"),
                (74, "Home"),
                (75, "PageUp"),
                (76, "Delete"),
                (77, "End"),
                (78, "PageDown"),
                (79, "ArrowRight"),
                (80, "ArrowLeft"),
                (81, "ArrowDown"),
                (82, "ArrowUp"),
                (83, "Num Lock"),
                (84, "Num /"),
                (85, "Num *"),
                (86, "Num -"),
                (87, "Num +"),
                (88, "Num Enter"),
                (89, "Num 1"),
                (90, "Num 2"),
                (91, "Num 3"),
                (92, "Num 4"),
                (93, "Num 5"),
                (94, "Num 6"),
                (95, "Num 7"),
                (96, "Num 8"),
                (97, "Num 9"),
                (98, "Num 0"),
                (99, "Num ."),
                (100, "ISO / ABNT"),
                (101, "Menu"),
                (135, "ABNT /"),
            ]
            .map(|(id, n)| (id, n.to_string())),
        );
        out
    })
}

fn key_label(name: &str) -> &str {
    match name {
        "Escape" => "Esc",
        "Space" => "Espaço",
        "ArrowRight" => "Seta direita",
        "ArrowLeft" => "Seta esquerda",
        "ArrowDown" => "Seta para baixo",
        "ArrowUp" => "Seta para cima",
        "OpenBracket" => "[",
        "CloseBracket" => "]",
        "Minus" => "-",
        "Equals" => "=",
        "Backslash" => "\\",
        "Semicolon" => ";",
        "Quote" => "'",
        "Comma" => ",",
        "Period" => ".",
        "Slash" => "/",
        _ => name,
    }
}

fn key_name(id: u8) -> String {
    choices()
        .iter()
        .find(|(code, _)| *code == id)
        .map(|(_, n)| tr(key_label(n)).into())
        .unwrap_or_else(|| format!("HID {id:02X}"))
}

fn mods(ui: &mut egui::Ui, bits: &mut u8, extended: bool) {
    for row in 0..if extended { 2 } else { 1 } {
        ui.horizontal_wrapped(|ui| {
            if extended {
                ui.label(tr(if row == 0 { "Esquerda" } else { "Direita" }));
            }
            for (i, text) in ["Ctrl", "Shift", "Alt", "Win"].into_iter().enumerate() {
                let b = 1 << (row * 4 + i);
                let mut enabled = *bits & b != 0;
                if ui.checkbox(&mut enabled, text).changed() {
                    if enabled {
                        *bits |= b;
                    } else {
                        *bits &= !b;
                    }
                }
            }
        });
    }
}

fn chord(ui: &mut egui::Ui, title: &str, names: &[String], mask: &mut u8) -> bool {
    let old = *mask;
    ui.label(RichText::new(title).strong());
    ui.horizontal_wrapped(|ui| {
        for (i, name) in names.iter().enumerate() {
            if ui.selectable_label(*mask & (1 << i) != 0, name).clicked() {
                *mask ^= 1 << i;
            }
        }
        if ui.small_button(tr("Limpar")).clicked() {
            *mask = 0;
        }
    });
    old != *mask
}

fn rgb_wheel(ui: &mut egui::Ui, light: &mut device::RgbLighting) {
    const SEGMENTS: usize = 72;
    let (area, response) =
        ui.allocate_exact_size(Vec2::splat(138.0), egui::Sense::click_and_drag());
    let center = area.center();
    let radius = 63.0;
    let display_color =
        |hue, saturation| Color32::from(egui::ecolor::Hsva::new(hue, saturation, 1.0, 1.0));
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, Color32::WHITE);
    for index in 0..=SEGMENTS {
        let hue = index as f32 / SEGMENTS as f32;
        mesh.colored_vertex(
            center + Vec2::angled(hue * std::f32::consts::TAU) * radius,
            display_color(hue, 1.0),
        );
        if index > 0 {
            mesh.add_triangle(0, index as u32, index as u32 + 1);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    if let Some(pointer) = response
        .interact_pointer_pos()
        .filter(|_| response.clicked() || response.dragged())
    {
        let vector = pointer - center;
        let hue = vector.angle().rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
        light.hue = (hue * 255.0).round() as u8;
        light.saturation = ((vector.length() / radius).clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    let choice = center
        + Vec2::angled(f32::from(light.hue) / 255.0 * std::f32::consts::TAU)
            * (f32::from(light.saturation) / 255.0 * radius);
    ui.painter()
        .circle_stroke(choice, 6.0, egui::Stroke::new(3.0_f32, BG));
    ui.painter()
        .circle_stroke(choice, 6.0, egui::Stroke::new(1.2_f32, Color32::WHITE));
}

impl App {
    pub(super) fn keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        if self.recording
            || self.confirm.is_some()
            || self.custom_dialog.is_some()
            || self.usb_dialog.is_some()
            || self.countdown.is_some()
            || ctx.data(|data| {
                data.get_temp::<(String, u8)>(egui::Id::new(DELETE_LAYOUT))
                    .is_some()
            })
        {
            return;
        }
        let command = ctx.input_mut(|input| {
            [
                (egui::Key::Num1, 1),
                (egui::Key::Num2, 2),
                (egui::Key::Num3, 3),
                (egui::Key::Num4, 4),
            ]
            .into_iter()
            .find_map(|(key, target)| {
                input
                    .consume_key(egui::Modifiers::ALT, key)
                    .then_some(target)
            })
            .or_else(|| {
                input
                    .consume_key(egui::Modifiers::CTRL, egui::Key::S)
                    .then_some(0)
            })
        });
        match command {
            Some(1) => self.page = Page::Keys,
            Some(2) => self.page = Page::Gestures,
            Some(3) => self.page = Page::Lights,
            Some(4) => {
                self.scan(ctx);
                self.usb_dialog = Some(device::usb_candidates());
            }
            Some(0) if !self.devices.is_empty() => match self.page {
                Page::Keys => {
                    let result = self.apply_binding();
                    self.result(result);
                }
                Page::Gestures => self.request_gesture_apply(ctx),
                Page::Lights
                    if self
                        .layout
                        .as_ref()
                        .is_some_and(|layout| layout.led_modes > 0) =>
                {
                    let result = self.apply_led();
                    self.result(result);
                }
                Page::Lights => {}
            },
            _ => {}
        }
    }

    pub(super) fn render(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .frame(egui::Frame::new().fill(PANEL).inner_margin(13))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if let Some(input) = &self.last_input {
                        ui.label(
                            RichText::new(format!("{} · {input}", tr("Última entrada")))
                                .color(BLUE),
                        );
                        ui.separator();
                    } else {
                        hint(
                            ui,
                            format!(
                                "{} · {}",
                                tr("Última entrada"),
                                tr("Aguardando o tecladinho")
                            ),
                        );
                        ui.separator();
                    }
                    hint(ui, &self.status);
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(20))
            .show(ctx, |ui| {
                self.header(ui);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.selectors(ui);
                    if let Some(layout) = self.layout.clone() {
                        match self.page {
                            Page::Keys if ui.available_width() > 690.0 => ui.columns(2, |col| {
                                card(&mut col[0], |ui| self.map(ui, &layout));
                                card(&mut col[1], |ui| self.editor(ui, &layout));
                                self.rotary_volume_card(&mut col[1]);
                            }),
                            Page::Keys => {
                                card(ui, |ui| self.map(ui, &layout));
                                ui.add_space(8.0);
                                card(ui, |ui| self.editor(ui, &layout));
                                self.rotary_volume_card(ui);
                            }
                            Page::Gestures => self.gesture_page(ui, &layout),
                            Page::Lights => self.lighting(ui, &layout),
                        }
                    } else {
                        card(ui, |ui| {
                            ui.heading(tr("Nenhum tecladinho conectado"));
                            hint(
                                ui,
                                tr("O modelo e os controles aparecem quando você conecta o USB."),
                            );
                        });
                    }
                });
            });
        if let Some(op) = self.confirm {
            egui::Modal::new(egui::Id::new("confirmar configuração"))
                .show(ctx, |ui| {
                    ui.set_max_width(453.0);
                    ui.heading(tr("Configuração do aparelho"));
                    ui.label(if op == Confirm::Activate {
                        tr("Aplicar as combinações e enviar os sinais ao tecladinho?")
                    } else {
                        tr("Gravar de volta as ações individuais que você cadastrou no perfil?")
                    });
                    if op == Confirm::Activate {
                        let unknown = {
                            let keys = self.gesture_slots();
                            let mask = gesture_targets(self.gesture_draft, ((1u16 << keys.len()) - 1) as u8).unwrap_or(0);
                            self.profile().map(|profile| missing_individual_actions(profile, &keys, mask))
                                .unwrap_or_default().into_iter().map(slot_name).collect::<Vec<_>>()
                        };
                        if !unknown.is_empty() {
                            ui.colored_label(CORAL, i18n::formatted(
                                "Salve primeiro as ações individuais de {} na aba Teclado.",
                                &[&unknown.join(", ")],
                            ));
                        } else {
                            hint(ui, tr("As ações individuais salvas continuam disponíveis ao pressionar uma tecla sozinha."));
                        }
                        if self.gesture_draft.enabled && !self.simulate_draft {
                            ui.label(system_text(if self.gesture_draft.shutdown_armed {
                                if self.shutdown_trial.is_complete() {
                                    "O atalho da tela e o desligamento do Windows estarão ativos."
                                } else {
                                    "O atalho da tela estará ativo. O primeiro gesto de desligar fará um teste de cinco segundos."
                                }
                            } else {
                                "O atalho da tela estará ativo. O desligamento do Windows continuará em simulação."
                            }));
                        }
                    }
                    ui.horizontal(|ui| {
                        if ui.button(if op == Confirm::Activate { tr("Confirmar envio") } else { tr("Restaurar") }).clicked() {
                            self.confirm = None;
                            let result = self.alter_host(ctx, op);
                            self.result(result);
                        }
                        if ui.button(tr("Voltar")).clicked() {
                            self.confirm = None;
                        }
                    });
                });
        }
        let report = self.devices.get(self.current).map(|device| {
            (
                device::report_ids(device),
                self.profile().and_then(|profile| profile.report_id),
            )
        });
        if let Some(requested) = custom::usb_dialog(ctx, &mut self.usb_dialog, report) {
            let result = self.test_usb_report(requested);
            self.result(result);
            let feedback = i18n::status(&self.status);
            ctx.data_mut(|data| data.insert_temp(egui::Id::new("usb-report-feedback"), feedback));
        }
        let remove_id = egui::Id::new(DELETE_LAYOUT);
        if let Some((profile, index)) = ctx.data(|data| data.get_temp::<(String, u8)>(remove_id)) {
            let modal = egui::Modal::new(remove_id).show(ctx, |ui| {
                ui.heading(tr("Excluir layout"));
                ui.label(tr("Excluir este layout salvo?"));
                ui.horizontal(|ui| {
                    let cancel = ui.button(tr("Cancelar")).clicked();
                    let remove = ui
                        .button(RichText::new(tr("Excluir layout")).color(CORAL))
                        .clicked();
                    (cancel, remove)
                })
                .inner
            });
            let (cancel, remove) = modal.inner;
            let still_selected = self.key.as_deref() == Some(profile.as_str());
            if cancel || remove || !still_selected || modal.should_close() {
                ctx.data_mut(|data| data.remove::<(String, u8)>(remove_id));
            }
            if remove && still_selected {
                let result = self.delete_visual_layout(index);
                self.result(result);
            }
        }
        let pressed = if self.custom_dialog.is_some() {
            (1..=21).fold(0u32, |mask, slot| {
                mask | (u32::from(self.physical_slot_down(slot)) << slot)
            })
        } else {
            0
        };
        let angles = [0, 1, 2].map(|index| self.dial_angle(index));
        if let Some(layout) = custom::layout_dialog(ctx, &mut self.custom_dialog, pressed, angles) {
            let index = self.edit_visual_slot;
            self.profile_mut().visual_slots[index as usize - 1] = Some(layout);
            self.profile_mut().visual_slot = index;
            self.restore_visual_layout();
            self.page = Page::Keys;
            self.slot = self
                .layout
                .as_ref()
                .and_then(|layout| {
                    layout
                        .controls
                        .iter()
                        .flat_map(|control| control.slots())
                        .find(|slot| self.allows_visual_slot(*slot))
                })
                .unwrap_or(1);
            self.layer = 1;
            self.reload_binding();
            let result = self
                .persist()
                .map(|_| tr("Layout personalizado salvo.").into());
            self.result(result);
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let compact = ui.available_width() < 730.0;
        ui.horizontal(|ui| {
            ui.add(egui::Image::new((
                self.logo.id(),
                Vec2::splat(if compact { 62.0 } else { 82.0 }),
            )));
            ui.heading(RichText::new(tr(APP_TITLE)).strong().size(if compact {
                21.0
            } else {
                28.0
            }));
            if !compact {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.header_actions(ui);
                });
            }
        });
        if compact {
            ui.horizontal_wrapped(|ui| self.header_actions(ui));
        }
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.page, Page::Keys, tr("Teclado"))
                .on_hover_text("Alt+1");
            ui.selectable_value(&mut self.page, Page::Gestures, tr("Combinações"))
                .on_hover_text("Alt+2");
            ui.selectable_value(&mut self.page, Page::Lights, tr("Iluminação"))
                .on_hover_text("Alt+3");
        });
        ui.separator();
        ui.add_space(5.0);
    }

    fn header_actions(&mut self, ui: &mut egui::Ui) {
        if ui
            .button(tr("Identificar USB"))
            .on_hover_text("Alt+4")
            .clicked()
        {
            self.scan(ui.ctx());
            self.usb_dialog = Some(device::usb_candidates());
        }
        let current = self.language.map_or_else(
            || system_text("Idioma do Windows"),
            |index| tr(i18n::LANGUAGES[index].1).into(),
        );
        let mut choice = self.language;
        egui::ComboBox::from_id_salt("interface-language")
            .selected_text(current)
            .width(118.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut choice, None, system_text("Idioma do Windows"));
                for (index, (_, name)) in i18n::LANGUAGES.iter().enumerate() {
                    ui.selectable_value(&mut choice, Some(index), tr(name));
                }
            })
            .response
            .on_hover_text(tr("Idioma"));
        if choice != self.language {
            match i18n::save(ui.ctx(), choice) {
                Ok(()) => {
                    self.language = choice;
                    #[cfg(not(windows))]
                    if let Some(listener) = &self.listener {
                        listener.refresh_tray_language();
                    }
                }
                Err(error) => self.result(Err(error)),
            }
        }
    }

    fn selectors(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let selected = self.profile().map_or(0, |profile| profile.visual_slot);
            let hidden = self.profile().is_some_and(|profile| profile.hide_default_layout);
            for index in 0..=2 {
                if index == 0 && hidden {
                    continue;
                }
                let name = match index {
                    0 => tr("Padrão"),
                    1 => tr("Layout 2"),
                    _ => tr("Layout 3"),
                };
                let entry = index > 0
                    && self
                        .profile()
                        .is_some_and(|profile| profile.visual_slots[index as usize - 1].is_some());
                if index == 0 || entry {
                    if ui.selectable_label(selected == index, name).clicked() && selected != index {
                        self.activate_visual_slot(index);
                    }
                } else if ui.button(format!("{} · {}", name, tr("Criar"))).clicked() {
                    self.edit_visual_slot = index;
                    let template = self
                        .devices
                        .get(self.current)
                        .map(|device| self.hardware_layout(device))
                        .unwrap_or_else(device::default_layout);
                    self.custom_dialog = Some(custom::CustomLayout::from_layout(&template));
                }
            }
            if let Some(layout) = self.profile().and_then(Profile::selected_layout).cloned() {
                if ui.button(tr("Editar layout")).clicked() {
                    self.edit_visual_slot = selected;
                    self.custom_dialog = Some(layout);
                }
                if ui.button(tr("Excluir layout")).clicked() {
                    if let Some(profile) = self.key.clone() {
                        ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new(DELETE_LAYOUT), (profile, selected)));
                    }
                }
            }
            if self.profile().and_then(Profile::first_saved_layout).is_some() {
                ui.menu_button("...", |ui| {
                    let label = if hidden { tr("Mostrar layout padrão") } else { tr("Ocultar layout padrão") };
                    if ui.button(label).clicked() {
                        self.set_default_layout_hidden(!hidden);
                        ui.close();
                    }
                }).response.on_hover_text(tr("Layout"));
            }
            ui.separator();
            let mut enabled = self.startup_enabled;
            if ui.checkbox(&mut enabled, system_text("Iniciar com o Windows na bandeja"))
                .on_hover_text(system_text("Na entrada do Windows, inicia na bandeja ou minimizado até o teclado conectar."))
                .changed()
            {
                match platform::set_startup(enabled) {
                    Ok(()) => {
                        self.startup_enabled = enabled;
                        self.status = if enabled {
                            tr("Inicialização automática ativada.")
                        } else {
                            tr("Inicialização automática desativada.")
                        }.into();
                    }
                    Err(error) => self.result(Err(error)),
                }
            }
        });
        if self.devices.len() > 1 {
            let old = self.current;
            egui::ComboBox::from_id_salt("usb")
                .selected_text(format!("{} {}", tr("Aparelho"), self.current + 1))
                .show_ui(ui, |ui| {
                    for (i, d) in self.devices.iter().enumerate() {
                        ui.selectable_value(
                            &mut self.current,
                            i,
                            format!("{} · {} · {}", i + 1, d.model, device_key(d)),
                        );
                    }
                });
            if old != self.current {
                self.select(ui.ctx());
            }
        }
        let Some(device) = self.devices.get(self.current) else {
            return;
        };
        let layouts = device::layouts_for(device);
        if layouts.len() > 1
            && self
                .profile()
                .is_some_and(|p| p.visual_slot == 0 && p.host == 0)
        {
            let before = self
                .layout
                .as_ref()
                .map(|l| l.name.clone())
                .unwrap_or_default();
            let mut choice = before.clone();
            egui::ComboBox::from_id_salt("variants")
                .selected_text(tr("Aparelho"))
                .show_ui(ui, |ui| {
                    for l in &layouts {
                        ui.selectable_value(&mut choice, l.name.clone(), i18n::model(&l.name));
                    }
                });
            if before != choice {
                self.layout = layouts.into_iter().find(|l| l.name == choice);
                self.profile_mut().layout = choice;
                self.layer = 1;
                self.slot = 1;
                self.reload_binding();
                if let Err(e) = self.persist() {
                    self.status = e;
                }
            }
        }
    }

    fn map(&mut self, ui: &mut egui::Ui, l: &Layout) {
        ui.heading(tr("Controles"));
        let before = (self.layer, self.slot);
        self.draw_board(ui, l);
        ui.label(RichText::new(slot_name(self.slot)).strong().color(BLUE));
        if l.layers > 1 {
            egui::ComboBox::from_id_salt("layer")
                .selected_text(format!("{} {}", tr("Camada"), self.layer))
                .show_ui(ui, |ui| {
                    for i in 1..=l.layers {
                        ui.selectable_value(&mut self.layer, i, format!("{} {i}", tr("Camada")));
                    }
                });
        }
        if before != (self.layer, self.slot) {
            self.reload_binding();
        }
        ui.separator();
        hint(ui, tr("Clique em um controle para editar a ação."));
    }

    fn draw_board(&mut self, ui: &mut egui::Ui, l: &Layout) {
        if l.controls.is_empty() {
            return;
        }
        let left = l.controls.iter().map(|c| c.x).fold(f32::INFINITY, f32::min) - 5.0;
        let top = l.controls.iter().map(|c| c.y).fold(f32::INFINITY, f32::min) - 5.0;
        let xmax = l
            .controls
            .iter()
            .map(|c| c.x - left + c.width + 5.0)
            .fold(1.0, f32::max);
        let ymax = l
            .controls
            .iter()
            .map(|c| c.y - top + c.height + 5.0)
            .fold(1.0, f32::max);
        let w = ui.available_width();
        let size = Vec2::new(w, (w * ymax / xmax).clamp(98.0, 266.0));
        let (area, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        ui.painter()
            .rect_filled(area, 11, Color32::from_rgb(13, 24, 39));
        let scale = (size.x / xmax).min(size.y / ymax);
        let origin = area.center() - Vec2::new(xmax, ymax) * (scale / 2.0);
        let painter = ui.painter_at(area);
        for control in &l.controls {
            let pos = origin + Vec2::new(control.x - left, control.y - top) * scale;
            let bounds = Rect::from_min_size(pos, Vec2::new(control.width, control.height) * scale);
            if control.kind == ControlKind::Button {
                let slot = control.id;
                if self.allows_visual_slot(slot) {
                    let motion = self.board_motion(ui, bounds, slot);
                    board_art::key(&painter, bounds, slot, motion);
                }
                continue;
            }

            let dial = board_art::Dial::new(bounds);
            let mut motion = [board_art::Motion::default(); 3];
            for (index, (slot, target)) in
                control.slots().into_iter().zip(dial.targets()).enumerate()
            {
                if !self.allows_visual_slot(slot) {
                    continue;
                }
                motion[index] = self.board_motion(ui, target, slot);
                if index != 1 {
                    dial.arrow(&painter, index == 0, motion[index]);
                }
            }
            let preview = (motion[2].hover - motion[0].hover) * 0.23;
            let angle = self.dial_angle(control.id - 1) + preview;
            dial.body(&painter, control.id, motion[1], angle);
        }
    }

    fn board_motion(&mut self, ui: &egui::Ui, target: Rect, slot: u8) -> board_art::Motion {
        let id = ui.make_persistent_id(("board-control", slot));
        let response = ui.interact(target, id, egui::Sense::click());
        if response.clicked() {
            self.slot = slot;
        }
        let selected = self.slot == slot;
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, slot_name(slot))
        });
        let response = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(slot_name(slot));
        board_art::Motion::animate(
            ui.ctx(),
            id,
            response.hovered(),
            response.is_pointer_button_down_on() || self.physical_slot_down(slot),
            selected,
            response.has_focus() && !response.hovered(),
        )
    }

    fn editor(&mut self, ui: &mut egui::Ui, l: &Layout) {
        let max_wheel = self
            .devices
            .get(self.current)
            .map_or(1, |device| device::wheel_steps(device, self.slot));
        let fd = self
            .devices
            .get(self.current)
            .is_some_and(|d| d.protocol == device::Protocol::Fd);
        ui.heading(tr("Ação"));
        if self
            .devices
            .get(self.current)
            .is_some_and(device::supports_rgb)
            && ui.small_button(tr("Ler ações do aparelho")).clicked()
        {
            let result = self.read_device_bindings();
            self.result(result);
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .selectable_label(matches!(self.draft, Binding::Keyboard(_)), tr("Teclado"))
                .clicked()
            {
                self.draft = seed();
            }
            if ui
                .add_enabled(
                    !fd,
                    egui::Button::selectable(matches!(self.draft, Binding::Media(_)), tr("Mídia")),
                )
                .on_disabled_hover_text(tr("Mídia direta indisponível no protocolo FD."))
                .clicked()
            {
                self.draft = Binding::Media(0xcd);
            }
            if ui
                .selectable_label(matches!(self.draft, Binding::Mouse { .. }), tr("Mouse"))
                .clicked()
            {
                self.draft = Binding::Mouse {
                    buttons: u8::from(!fd),
                    scroll: i8::from(fd),
                    modifiers: 0,
                };
            }
        });
        match &mut self.draft {
            Binding::Keyboard(steps) => {
                let max = l.max_sequence.max(1) as usize;
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .toggle_value(&mut self.recording, tr("Gravar teclas"))
                        .changed()
                        && self.recording
                    {
                        steps.clear();
                    }
                    if ui
                        .add_enabled(steps.len() < max, egui::Button::new(tr("Adicionar passo")))
                        .clicked()
                    {
                        steps.push((0, 4));
                    }
                    if ui.small_button(tr("Limpar")).clicked() {
                        steps.clear();
                    }
                });
                if self.recording {
                    for event in ui.input(|i| i.events.clone()) {
                        if let egui::Event::Key {
                            key,
                            physical_key,
                            pressed: true,
                            repeat: false,
                            modifiers,
                        } = event
                        {
                            let label = format!("{:?}", physical_key.unwrap_or(key));
                            let label = label.strip_prefix("Num").unwrap_or(&label);
                            if let Some((code, _)) =
                                choices().iter().find(|(_, name)| name == label)
                            {
                                if steps.len() < max {
                                    let mask = modifiers.ctrl as u8
                                        | ((modifiers.shift as u8) << 1)
                                        | ((modifiers.alt as u8) << 2)
                                        | ((modifiers.mac_cmd as u8) << 3);
                                    steps.push((mask, *code));
                                }
                            }
                        }
                    }
                    if steps.len() >= max {
                        self.recording = false;
                    }
                }
                let mut delete = None;
                for (i, (bits, key)) in steps.iter_mut().enumerate() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("{} {}", tr("Passo"), i + 1));
                        egui::ComboBox::from_id_salt(("key", i))
                            .selected_text(key_name(*key))
                            .show_ui(ui, |ui| {
                                for (id, name) in choices() {
                                    ui.selectable_value(key, *id, tr(key_label(name)));
                                }
                            });
                        if ui.small_button(tr("Remover")).clicked() {
                            delete = Some(i);
                        }
                    });
                    mods(ui, bits, l.supports_delay);
                }
                if let Some(i) = delete {
                    steps.remove(i);
                }
                if l.supports_delay {
                    ui.horizontal(|ui| {
                        ui.label(tr("Intervalo"));
                        ui.add(egui::DragValue::new(&mut self.delay).suffix(" ms"));
                    });
                } else if steps.len() > 1 {
                    hint(
                        ui,
                        tr("No aparelho, os modificadores podem valer apenas no primeiro passo."),
                    );
                }
            }
            Binding::Media(code) => {
                ui.horizontal_wrapped(|ui| {
                    for (name, id) in MEDIA {
                        if ui
                            .add(egui::Button::selectable(*code == id, tr(name)))
                            .clicked()
                        {
                            *code = id;
                        }
                    }
                });
            }
            Binding::Mouse {
                buttons,
                scroll,
                modifiers,
            } => {
                ui.horizontal_wrapped(|ui| {
                    for (name, b, s) in MOUSE {
                        if fd && s == 0 {
                            continue;
                        }
                        if ui
                            .add(egui::Button::selectable(
                                *buttons == b && scroll.signum() == s,
                                tr(name),
                            ))
                            .clicked()
                        {
                            *buttons = b;
                            *scroll = s * i8::try_from(scroll.unsigned_abs())
                                .unwrap_or(1)
                                .clamp(1, max_wheel);
                        }
                    }
                });
                if *scroll != 0 && max_wheel > 1 {
                    let mut steps = scroll.unsigned_abs();
                    ui.horizontal(|ui| {
                        ui.label(tr("Passos de rolagem por clique"));
                        if ui
                            .add(egui::Slider::new(&mut steps, 1..=max_wheel as u8))
                            .changed()
                        {
                            *scroll = scroll.signum() * steps as i8;
                        }
                    });
                }
                mods(ui, modifiers, l.supports_delay);
            }
        }
        ui.separator();
        let managed = self.manages_binding(self.layer, self.slot);
        if managed {
            hint(
                ui,
                tr("Ação individual em modo combinado. Salvar aqui mantém os sinais das combinações."),
            );
        }
        if ui
            .add_enabled(
                !self.devices.is_empty(),
                egui::Button::new(if managed {
                    tr("Salvar ação individual")
                } else {
                    tr("Aplicar ação selecionada")
                }),
            )
            .clicked()
        {
            let r = self.apply_binding();
            self.result(r);
        }
    }

    fn rotary_volume_card(&mut self, ui: &mut egui::Ui) {
        let supported = self.devices.get(self.current).is_some_and(|device| {
            device.protocol != device::Protocol::Fd
                && self.hardware_layout(device).controls.iter().any(|control| {
                    control
                        .slots()
                        .iter()
                        .any(|slot| platform::volume_key(*slot).is_some())
                })
        });
        if supported {
            ui.add_space(8.0);
            card(ui, |ui| self.rotary_volume(ui));
        }
    }

    fn rotary_volume(&mut self, ui: &mut egui::Ui) {
        let Some(profile) = self.profile() else {
            return;
        };
        let (mut step, managed) = (profile.volume_step, profile.volume_mode);
        ui.heading(tr("Volume do giratório"));
        ui.horizontal_wrapped(|ui| {
            for (enabled, label) in [
                (false, tr("Direto no teclado")),
                (true, tr("Passo do aplicativo")),
            ] {
                if ui.selectable_label(managed == enabled, label).clicked() {
                    let result = self.set_volume_mode(enabled);
                    self.result(result);
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            if self.profile().is_some_and(|profile| profile.volume_mode) {
                ui.label(tr("Passo por giro"));
                if ui
                    .add(egui::Slider::new(&mut step, 1..=10).suffix("%"))
                    .changed()
                {
                    let result = self.set_volume_step(step);
                    self.result(result);
                }
            }
            if ui.small_button(tr("Conferir volume")).clicked() {
                let result = audio::current().map(|level| {
                    self.volume_osd.show(level);
                    format!("{}: {level}%.", system_text("Volume do Windows"))
                });
                self.result(result);
            }
        });
        hint(
            ui,
            if managed {
                tr("O mesmo passo vale para todos os giros configurados como volume. O aplicativo pode ficar na bandeja.").into()
            } else {
                system_text(
                    "O volume direto usa o passo do Windows e funciona com o aplicativo fechado.",
                )
            },
        );
    }

    fn gesture_page(&mut self, ui: &mut egui::Ui, l: &Layout) {
        if ui.available_width() > 830.0 {
            ui.columns(2, |columns| {
                self.gesture_settings(&mut columns[0], l);
                self.host_controls(&mut columns[1]);
            });
        } else {
            self.gesture_settings(ui, l);
            ui.add_space(8.0);
            self.host_controls(ui);
        }
    }

    fn gesture_settings(&mut self, ui: &mut egui::Ui, _layout: &Layout) {
        let names = self
            .gesture_slots()
            .iter()
            .map(|s| slot_name(*s))
            .collect::<Vec<_>>();
        let mut cfg = self.gesture_draft;
        let host = self.profile().map_or(0, |p| p.host);
        card(ui, |ui| {
            ui.heading(tr("Combinações"));
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut cfg.enabled, tr("Ativar combinações"));
                ui.checkbox(&mut self.simulate_draft, tr("Modo de teste"));
            });
            ui.add_enabled(
                !self.simulate_draft && cfg.shutdown != 0,
                egui::Checkbox::new(
                    &mut cfg.shutdown_armed,
                    tr("Permitir desligamento real do PC"),
                ),
            );
            if cfg.shutdown != 0 && !self.shutdown_trial.is_complete() {
                hint(
                    ui,
                    tr("O primeiro gesto de desligar sempre roda um teste de cinco segundos. Termine ou cancele para liberar o modo real."),
                );
            }
            hint(
                ui,
                tr("Escolha de uma a quatro teclas por ação. Limpar desativa esse gesto."),
            );
            ui.separator();
            chord(ui, tr("Ligar / apagar a tela"), &names, &mut cfg.display);
            chord(
                ui,
                tr("Desligar computador em 5 segundos"),
                &names,
                &mut cfg.shutdown,
            );
            chord(ui, tr("Cancelar desligamento"), &names, &mut cfg.cancel);
            if cfg.display != 0 && cfg.display == cfg.shutdown {
                ui.colored_label(
                    CORAL,
                    tr("Escolha gestos diferentes para apagar a tela e desligar."),
                );
            }
            if host != 0 && (cfg.display | cfg.shutdown | cfg.cancel) & !host != 0 {
                hint(
                    ui,
                    tr("Use Aplicar e enviar para registrar os novos controles."),
                );
            }
            if ui
                .add_enabled(
                    self.engine.remaining(Instant::now()).is_none(),
                    egui::Button::new(tr("Demonstrar contagem de 5 segundos")),
                )
                .clicked()
            {
                self.preview_until = Some(Instant::now() + Duration::from_secs(5));
            }
            let held = self.engine.pressed_mask();
            if host != 0 {
                let pressed = names
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| held & (1 << i) != 0)
                    .map(|(_, n)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(" + ");
                hint(
                    ui,
                    if pressed.is_empty() {
                        format!("{}: {}", tr("Entradas pressionadas"), tr("nenhuma"))
                    } else {
                        format!("{}: {pressed}", tr("Entradas pressionadas"))
                    },
                );
            }
        });
        self.gesture_draft = cfg;
    }

    fn request_gesture_apply(&mut self, ctx: &egui::Context) {
        let available = ((1u16 << self.gesture_slots().len()) - 1) as u8;
        match gesture_targets(self.gesture_draft, available) {
            Ok(0) => {
                let result = self.alter_host(ctx, Confirm::Activate);
                self.result(result);
            }
            Ok(targets) => {
                let missing = missing_individual_actions(
                    self.profile().unwrap(),
                    &self.gesture_slots(),
                    targets,
                );
                if missing.is_empty() {
                    self.confirm = Some(Confirm::Activate);
                } else {
                    let names = missing
                        .into_iter()
                        .map(slot_name)
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.result(Err(format!(
                        "Salve as ações individuais em Teclado. Faltam {names}."
                    )));
                }
            }
            Err(error) => self.result(Err(error)),
        }
    }

    fn host_controls(&mut self, ui: &mut egui::Ui) {
        let host = self.profile().map_or(0, |p| p.host);
        card(ui, |ui| {
            ui.heading(tr("Enviar ao tecladinho"));
            hint(ui,system_text("O aparelho envia os sinais das teclas. O aplicativo executa as combinações no Windows."));
            if ui
                .add_enabled(
                    !self.devices.is_empty(),
                    egui::Button::new(
                        RichText::new(tr("Aplicar e enviar"))
                            .strong()
                            .color(Color32::WHITE),
                    )
                    .fill(Color32::from_rgb(28, 78, 127)),
                )
                .clicked()
            {
                self.request_gesture_apply(ui.ctx());
            }
            if host != 0 {
                ui.separator();
                hint(
                    ui,
                    if self.simulate {
                        tr("Aplicado · modo de teste")
                    } else {
                        tr("Aplicado · ações reais")
                    },
                );
                if ui
                    .checkbox(&mut self.paused, tr("Pausar combinações"))
                    .changed()
                {
                    self.engine.reset();
                }
                ui.horizontal_wrapped(|ui| {
                    if self.listener.is_none() && ui.button(tr("Iniciar escuta")).clicked() {
                        let result = self.listen(ui.ctx()).map(|_| "Escuta iniciada.".into());
                        self.result(result);
                    }
                    if ui.button(tr("Restaurar modo direto")).clicked() {
                        self.confirm = Some(Confirm::Restore);
                    }
                });
            }
        });
    }

    fn lighting(&mut self, ui: &mut egui::Ui, l: &Layout) {
        card(ui, |ui| {
            ui.heading(tr("Iluminação"));
            if self
                .devices
                .get(self.current)
                .is_some_and(device::supports_rgb)
            {
                self.lighting_rgb(ui);
                return;
            }
            if l.led_modes == 0 {
                hint(ui, tr("O perfil não informa modos de LED."));
                return;
            }
            ui.horizontal_wrapped(|ui| {
                for m in 0..l.led_modes {
                    ui.selectable_value(
                        &mut self.led,
                        m,
                        if m == 0 {
                            tr("Desligado").into()
                        } else {
                            format!("{} {m}", tr("Modo"))
                        },
                    );
                }
            });
            if l.supports_led_color {
                let colors = [
                    tr("Automática"),
                    tr("Vermelho"),
                    tr("Laranja"),
                    tr("Amarelo"),
                    tr("Verde"),
                    tr("Ciano"),
                    tr("Azul"),
                    tr("Roxo"),
                ];
                egui::ComboBox::from_id_salt("led-color")
                    .selected_text(tr(colors[(self.led_color >> 4) as usize]))
                    .show_ui(ui, |ui| {
                        for (i, n) in colors.into_iter().enumerate() {
                            ui.selectable_value(&mut self.led_color, (i as u8) << 4, tr(n));
                        }
                    });
            } else {
                hint(ui, tr("Este modelo oferece modos de iluminação. A escolha manual de cor depende do firmware e não está disponível no perfil reconhecido."));
            }
            if ui.button(tr("Aplicar iluminação")).clicked() {
                let r = self.apply_led();
                self.result(r);
            }
        });
    }

    fn lighting_rgb(&mut self, ui: &mut egui::Ui) {
        if ui.button(tr("Ler iluminação do aparelho")).clicked() {
            let result = self.read_rgb();
            self.result(result);
        }
        ui.horizontal_wrapped(|ui| {
            for (index, name) in [
                tr("Desligado"),
                tr("Fixa"),
                tr("Respiração"),
                tr("Ao pressionar"),
                tr("Onda"),
            ]
            .into_iter()
            .enumerate()
            {
                ui.selectable_value(&mut self.rgb.mode, index as u8, name);
            }
        });
        ui.horizontal_wrapped(|ui| {
            rgb_wheel(ui, &mut self.rgb);
            ui.vertical(|ui| {
                ui.checkbox(&mut self.rgb.palette, tr("Paleta animada"));
                ui.add(egui::Slider::new(&mut self.rgb.hue, 0..=255).text(tr("Matiz")));
                ui.add(egui::Slider::new(&mut self.rgb.saturation, 0..=255).text(tr("Saturação")));
                ui.add(egui::Slider::new(&mut self.rgb.value, 0..=255).text(tr("Brilho")));
                ui.add(egui::Slider::new(&mut self.rgb.speed, 0..=4).text(tr("Velocidade")));
            });
        });
        if ui.button(tr("Aplicar iluminação")).clicked() {
            let result = self.apply_led();
            self.result(result);
        }
    }
}
