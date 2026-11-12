//! Task navigation, focus, and local commands shared by the wizard shell.

use eframe::egui;
use portableforge_core::settings::{Appearance, TEXT_SCALES};

use super::{theme, WizardApp};

#[derive(Default)]
pub(super) struct State {
    page: Option<(bool, usize, bool)>,
    popup: Option<Dialog>,
    recent_commands: Vec<Command>,
    return_focus: Option<egui::Id>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Popup {
    Commands,
    Appearance,
    Help,
}

struct Dialog {
    kind: Popup,
    opener: Option<egui::Id>,
    query: String,
    selected: usize,
    focus_pending: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WizardAction {
    Continue,
    StartCapture,
    StartMsi,
    SaveProfile,
    BuildInstaller,
    Finish,
}

impl WizardAction {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Continue => "Continue",
            Self::StartCapture => "Start capture",
            Self::StartMsi => "Start Windows Sandbox",
            Self::SaveProfile => "Save profile",
            Self::BuildInstaller => "Build installer",
            Self::Finish => "Finish",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Primary(WizardAction),
    Back,
    Details,
    Library,
    Appearance,
    Help,
    FocusRegion,
    CancelJob,
}

struct Entry {
    command: Command,
    label: &'static str,
    keys: &'static str,
    keywords: &'static str,
    enabled: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Region {
    Header,
    Body,
    Footer,
}

impl Region {
    fn key(self) -> egui::Id {
        egui::Id::new(("wizard-focus-region", self as u8))
    }
}

fn heading_pending() -> egui::Id {
    egui::Id::new("wizard-heading-focus-pending")
}

pub(super) fn register_region(ui: &egui::Ui, region: Region, id: egui::Id) {
    ui.data_mut(|data| data.insert_temp(region.key(), id));
}

pub(super) fn register_heading(ui: &egui::Ui, response: &egui::Response) {
    register_region(ui, Region::Body, response.id);
    let pending = ui.data_mut(|data| {
        let pending = data.get_temp::<bool>(heading_pending()).unwrap_or(false);
        data.remove::<bool>(heading_pending());
        pending
    });
    if pending {
        response.request_focus();
    }
    theme::focus_outline(ui, response);
}

impl WizardApp {
    pub(super) fn update_heading_focus(&mut self, ctx: &egui::Context) {
        let page = (self.profile_flow.replay_open, self.step, self.show_details);
        if self.shell.page != Some(page) {
            self.shell.page = Some(page);
            ctx.data_mut(|data| {
                data.remove::<egui::Id>(Region::Footer.key());
                data.insert_temp(heading_pending(), true);
            });
        }
    }

    pub(super) fn current_wizard_action(&self) -> Option<(WizardAction, bool)> {
        if self.show_details || self.profile_flow.replay_open {
            return None;
        }
        let verified = self.use_verified_capture();
        Some(match self.step {
            2 if self.uses_guided_msi() && self.layout.is_none() => {
                (WizardAction::StartMsi, self.msi_capture_ready())
            }
            2 if verified && !self.profile_flow.captured => {
                (WizardAction::StartCapture, self.capture_ready())
            }
            9 if verified && !self.profile_flow.profile_saved() => {
                (WizardAction::SaveProfile, self.profile_save_ready())
            }
            11 if verified && self.profile_flow.profile_saved() => (
                WizardAction::BuildInstaller,
                self.profile_flow.saved_approval_ready() && !self.profile_flow.busy(),
            ),
            11 => (WizardAction::Finish, self.can_advance()),
            _ => (WizardAction::Continue, self.can_advance()),
        })
    }

    pub(super) fn run_wizard_action(&mut self, ctx: &egui::Context, action: WizardAction) {
        if self.current_wizard_action() != Some((action, true)) {
            return;
        }
        match action {
            WizardAction::StartCapture => self.start_verified_capture(ctx),
            WizardAction::StartMsi => self.start_msi_capture(ctx),
            WizardAction::SaveProfile => self.start_profile_save(ctx),
            WizardAction::BuildInstaller => self.open_profile_replay(true),
            WizardAction::Continue | WizardAction::Finish => {
                self.advance();
            }
        }
        ctx.request_repaint();
    }

    pub(super) fn show_phase_navigation(&mut self, ui: &mut egui::Ui, index: usize, total: usize) {
        let phases = if self.use_verified_capture() {
            [
                ("Capture", 0),
                ("Review", 6),
                ("Save profile", 9),
                ("Build", 11),
            ]
        } else {
            [("Capture", 0), ("Review", 6), ("Package", 9), ("Test", 10)]
        };
        let current = phases
            .iter()
            .rposition(|(_, start)| *start <= self.step)
            .unwrap_or(0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().interact_size.y = theme::compact_row_height();
            for (phase, (label, start)) in phases.into_iter().enumerate() {
                if phase > 0 {
                    ui.label(egui::RichText::new("›").weak());
                }
                let selected = phase == current;
                let response = ui.add_enabled(
                    selected || (phase < current && self.can_go_back() && !self.show_details),
                    egui::Button::selectable(selected, label),
                );
                if selected {
                    let y = response.rect.bottom() - 1.0;
                    ui.painter()
                        .hline(response.rect.x_range(), y, ui.visuals().selection.stroke);
                }
                if response
                    .on_hover_text(if selected {
                        format!("Step {index} of {total}")
                    } else {
                        label.to_owned()
                    })
                    .clicked()
                    && phase < current
                {
                    while self.step > start && self.can_go_back() {
                        self.go_back();
                    }
                    ui.ctx().request_repaint();
                }
            }
        });
    }

    pub(super) fn toggle_step_details(&mut self) {
        if !self.profile_flow.replay_open && !self.profile_flow.busy() {
            self.detail_page = 0;
            self.show_details = !self.show_details;
        }
    }

    pub(super) fn open_shell_popup(&mut self, kind: Popup, opener: Option<egui::Id>) {
        if self
            .shell
            .popup
            .as_ref()
            .is_some_and(|popup| popup.kind == kind)
        {
            return;
        }
        self.shell.popup = Some(Dialog {
            kind,
            opener,
            query: String::new(),
            selected: 0,
            focus_pending: true,
        });
    }

    pub(super) fn handle_shell_keys(&mut self, ctx: &egui::Context) {
        if self.shell.popup.is_some() || ctx.memory(|memory| memory.top_modal_layer().is_some()) {
            return;
        }
        let focused = ctx.memory(|memory| memory.focused());
        let in_editor = focused.is_some_and(|id| egui::TextEdit::load_state(ctx, id).is_some());
        let use_key = |modifiers, key| ctx.input_mut(|input| input.consume_key(modifiers, key));
        if use_key(egui::Modifiers::COMMAND, egui::Key::K) {
            self.open_shell_popup(Popup::Commands, focused);
        } else if use_key(egui::Modifiers::COMMAND, egui::Key::Comma) {
            self.open_shell_popup(Popup::Appearance, focused);
        } else if use_key(egui::Modifiers::NONE, egui::Key::F1) {
            if self.profile_flow.replay_open || self.profile_flow.busy() {
                self.open_shell_popup(Popup::Help, focused);
            } else {
                self.toggle_step_details();
            }
        } else if use_key(egui::Modifiers::NONE, egui::Key::F6) {
            focus_next_region(ctx);
        } else if !in_editor
            && (use_key(egui::Modifiers::ALT, egui::Key::ArrowLeft)
                || use_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.shell_back();
        } else if !in_editor
            && self.current_wizard_action() == Some((WizardAction::SaveProfile, true))
            && use_key(egui::Modifiers::COMMAND, egui::Key::S)
        {
            self.run_wizard_action(ctx, WizardAction::SaveProfile);
        } else if (focused.is_none() || focused == region_focus(ctx, Region::Body))
            && !in_editor
            && use_key(egui::Modifiers::NONE, egui::Key::Enter)
        {
            if let Some((action, true)) = self.current_wizard_action() {
                self.run_wizard_action(ctx, action);
            }
        }
    }

    fn shell_back(&mut self) {
        if self.profile_flow.busy() {
            return;
        }
        if self.profile_flow.replay_open {
            self.profile_flow.replay_open = false;
        } else if self.show_details {
            self.toggle_step_details();
        } else {
            self.go_back();
        }
    }

    fn command_entries(&self) -> Vec<Entry> {
        let busy = self.profile_flow.busy();
        let mut entries = vec![
            Entry {
                command: Command::Back,
                label: "Go back",
                keys: "Esc",
                keywords: "previous close",
                enabled: !busy
                    && (self.can_go_back() || self.show_details || self.profile_flow.replay_open),
            },
            Entry {
                command: Command::Details,
                label: "Show status and details",
                keys: "F1",
                keywords: "log help error",
                enabled: !busy && !self.profile_flow.replay_open,
            },
            Entry {
                command: Command::Library,
                label: "Open saved profiles",
                keys: "",
                keywords: "library queue build results",
                enabled: !busy && !self.profile_flow.replay_open,
            },
            Entry {
                command: Command::Appearance,
                label: "Change appearance",
                keys: "Ctrl+,",
                keywords: "settings theme text size",
                enabled: true,
            },
            Entry {
                command: Command::Help,
                label: "View keyboard shortcuts",
                keys: "",
                keywords: "help keys controls",
                enabled: true,
            },
            Entry {
                command: Command::FocusRegion,
                label: "Focus the next region",
                keys: "F6",
                keywords: "tab navigation",
                enabled: true,
            },
        ];
        if let Some((action, enabled)) = self.current_wizard_action() {
            entries.insert(
                0,
                Entry {
                    command: Command::Primary(action),
                    label: action.label(),
                    keys: if action == WizardAction::SaveProfile {
                        "Ctrl+S"
                    } else {
                        "Enter"
                    },
                    keywords: "next current task",
                    enabled,
                },
            );
        }
        if self.profile_flow.can_cancel_owned_job() {
            entries.insert(
                0,
                Entry {
                    command: Command::CancelJob,
                    label: "Cancel the current test box operation",
                    keys: "",
                    keywords: "stop capture test",
                    enabled: true,
                },
            );
        }
        entries
    }

    pub(super) fn show_shell_popup(&mut self, ctx: &egui::Context) {
        let Some(mut popup) = self.shell.popup.take() else {
            if let Some(opener) = self.shell.return_focus {
                if ctx.memory(|memory| memory.top_modal_layer().is_none()) {
                    ctx.memory_mut(|memory| memory.request_focus(opener));
                    self.shell.return_focus = None;
                }
                ctx.request_repaint();
            }
            return;
        };
        let dialog_id = egui::Id::new(match popup.kind {
            Popup::Commands => "wizard-commands",
            Popup::Appearance => "wizard-appearance",
            Popup::Help => "wizard-keyboard-help",
        });
        let response = egui::Modal::new(dialog_id).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 44.0).clamp(240.0, 460.0));
            let command = match popup.kind {
                Popup::Commands => self.show_commands(ui, &mut popup),
                Popup::Appearance => {
                    self.show_appearance(ui, &mut popup);
                    None
                }
                Popup::Help => {
                    ui.heading("Keyboard shortcuts");
                    for (key, detail) in [
                        ("Tab / Shift+Tab", "Move through controls"),
                        ("Ctrl+K", "Open commands"),
                        ("Ctrl+,", "Open appearance"),
                        ("F1", "Open status and details"),
                        ("F6", "Move to the next region"),
                        ("Enter", "Activate the focused action"),
                        ("Esc", "Close a dialog or go back"),
                    ] {
                        ui.label(format!("{key}  {detail}"));
                    }
                    None
                }
            };
            let close = ui.button("Close");
            if popup.focus_pending && popup.kind == Popup::Help {
                close.request_focus();
                popup.focus_pending = false;
            }
            (command, close.clicked())
        });
        let (chosen, closed) = response.inner;
        if chosen.is_some() || closed || response.should_close() {
            if let Some(command) = chosen {
                self.shell
                    .recent_commands
                    .retain(|previous| *previous != command);
                self.shell.recent_commands.insert(0, command);
                self.execute_command(ctx, command, popup.opener);
            } else {
                self.shell.return_focus = popup.opener;
                ctx.request_repaint();
            }
        } else {
            self.shell.popup = Some(popup);
        }
    }

    fn show_commands(&self, ui: &mut egui::Ui, popup: &mut Dialog) -> Option<Command> {
        ui.heading("Commands");
        let label = ui.label("Find an action");
        let search_id = ui.make_persistent_id("command-search");
        let focused = ui.memory(|memory| memory.has_focus(search_id));
        let (previous, next, submit) = if focused {
            ui.input_mut(|input| {
                (
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                    input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                )
            })
        } else {
            (false, false, false)
        };
        let editor = ui
            .add(
                egui::TextEdit::singleline(&mut popup.query)
                    .id(search_id)
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(label.id);
        if popup.focus_pending {
            editor.request_focus();
            popup.focus_pending = false;
        }
        if editor.changed() {
            popup.selected = 0;
        }
        let query = popup.query.trim().to_lowercase();
        let mut matches = self
            .command_entries()
            .into_iter()
            .filter_map(|entry| {
                command_rank(&query, entry.label, entry.keywords).map(|rank| (rank, entry))
            })
            .collect::<Vec<_>>();
        matches.sort_by_key(|(rank, entry)| {
            (
                !entry.enabled,
                self.shell
                    .recent_commands
                    .iter()
                    .position(|item| *item == entry.command)
                    .unwrap_or(usize::MAX),
                *rank,
            )
        });
        let enabled = matches.iter().filter(|(_, entry)| entry.enabled).count();
        popup.selected = popup.selected.min(enabled.saturating_sub(1));
        if enabled > 0 {
            if next {
                popup.selected = (popup.selected + 1) % enabled;
            }
            if previous {
                popup.selected = (popup.selected + enabled - 1) % enabled;
            }
        }
        let mut chosen = None;
        for (i, (_, entry)) in matches.iter().enumerate() {
            let text = if entry.keys.is_empty() {
                entry.label.to_owned()
            } else {
                format!("{}  ·  {}", entry.label, entry.keys)
            };
            let row = ui.add_enabled(
                entry.enabled,
                egui::Button::selectable(i == popup.selected && entry.enabled, text),
            );
            if row
                .on_disabled_hover_text("This action is unavailable at the current step.")
                .clicked()
            {
                chosen = Some(entry.command);
            }
        }
        if matches.is_empty() {
            ui.label("No commands match your search.");
        }
        if submit && popup.selected < enabled {
            chosen = Some(matches[popup.selected].1.command);
        }
        chosen
    }

    fn show_appearance(&mut self, ui: &mut egui::Ui, popup: &mut Dialog) {
        ui.heading("Appearance");
        ui.label("Theme");
        for (label, value) in [
            ("System", Appearance::System),
            ("GitHub Dark", Appearance::Dark),
            ("GitHub Light", Appearance::Light),
        ] {
            let response = ui.radio_value(&mut self.settings.appearance, value, label);
            if popup.focus_pending && self.settings.appearance == value {
                response.request_focus();
                popup.focus_pending = false;
            }
            if response.changed() {
                self.settings_dirty = true;
                self.persist_now();
                ui.ctx().request_repaint();
            }
        }
        ui.label("Text size");
        ui.horizontal_wrapped(|ui| {
            for (label, scale) in ["Standard", "Large", "Extra large"]
                .into_iter()
                .zip(TEXT_SCALES)
            {
                if ui
                    .selectable_label(self.settings.text_scale() == scale, label)
                    .clicked()
                {
                    self.settings.text_scale = scale;
                    self.settings_dirty = true;
                    self.persist_now();
                    ui.ctx().request_repaint();
                }
            }
        });
    }

    fn execute_command(&mut self, ctx: &egui::Context, command: Command, opener: Option<egui::Id>) {
        match command {
            Command::Primary(action) => self.run_wizard_action(ctx, action),
            Command::Back => self.shell_back(),
            Command::Details => self.toggle_step_details(),
            Command::Library => self.open_profile_replay(false),
            Command::Appearance => self.open_shell_popup(Popup::Appearance, opener),
            Command::Help => self.open_shell_popup(Popup::Help, opener),
            Command::FocusRegion => focus_next_region(ctx),
            Command::CancelJob => self.profile_flow.cancel_capture(),
        }
        ctx.request_repaint();
    }
}

fn region_focus(ctx: &egui::Context, region: Region) -> Option<egui::Id> {
    ctx.data(|data| data.get_temp::<egui::Id>(region.key()))
}

fn focus_next_region(ctx: &egui::Context) {
    let current = ctx.memory(|memory| memory.focused());
    let regions = [Region::Header, Region::Body, Region::Footer];
    let start = regions
        .iter()
        .position(|region| current.is_some() && region_focus(ctx, *region) == current)
        .unwrap_or(1);
    for offset in 1..=regions.len() {
        if let Some(next) = region_focus(ctx, regions[(start + offset) % regions.len()]) {
            ctx.memory_mut(|memory| memory.request_focus(next));
            break;
        }
    }
}

fn command_rank(query: &str, label: &str, keywords: &str) -> Option<u8> {
    let label = label.to_lowercase();
    if query.is_empty() || label.starts_with(query) {
        return Some(0);
    }
    let combined = format!("{label} {keywords}");
    if combined.contains(query) || query.split_whitespace().all(|part| combined.contains(part)) {
        return Some(1);
    }
    let mut remaining = label.chars();
    query
        .chars()
        .filter(|c| !c.is_whitespace())
        .all(|character| remaining.any(|next| character == next))
        .then_some(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_filter_matches_plain_words_and_subsequences() {
        assert_eq!(
            command_rank("open", "Open saved profiles", "library"),
            Some(0)
        );
        assert_eq!(
            command_rank("profiles library", "Open saved profiles", "library"),
            Some(1)
        );
        assert_eq!(
            command_rank("osp", "Open saved profiles", "library"),
            Some(2)
        );
        assert_eq!(
            command_rank("erase", "Open saved profiles", "library"),
            None
        );
    }

    #[test]
    fn escape_restores_the_dialog_opener_after_its_modal_layer_is_gone() {
        let ctx = egui::Context::default();
        let mut wizard = WizardApp::default();
        let mut opener = egui::Id::NULL;
        let mut draw = |open: bool, escape: bool| {
            let events = if escape {
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }]
            } else {
                Vec::new()
            };
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(640.0, 480.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        opener = ui.button("Appearance").id;
                        if open {
                            wizard.open_shell_popup(Popup::Appearance, Some(opener));
                        }
                    });
                    wizard.show_shell_popup(ctx);
                },
            );
        };
        draw(true, false);
        draw(false, false);
        draw(false, true);
        draw(false, false);
        draw(false, false);
        assert!(wizard.shell.popup.is_none());
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(opener));
    }
}
