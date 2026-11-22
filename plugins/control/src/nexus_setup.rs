use dorobot_studio_core::{
    api::{
        nexus::{NexusIntent, NexusKind},
        Backend, Intent,
    },
    ui::frame::{self, Themed},
};
use dorobot_studio_plugin::*;

pub struct NexusSetup {
    root: WidgetRef,
    populated: bool,
    light: Option<f64>,
}

impl NexusSetup {
    pub fn create(vm: &mut ScriptVm) -> Self {
        let value = script_eval!(vm, {
            use mod.prelude.widgets.*
            use mod.widgets.*
            let Setting = TextInput{width: Fill height: 36 min_height: 36}
            View{width: Fill height: Fill flow: Down spacing: 12.0 padding: 16.0
                Label{text: "SO-101 · Nexus simulation · Local host"
                    draw_text +: {text_style: mod.widgets.ux.TEXT_TITLE{}}}
                settings := mod.widgets.ux.Card{width: Fill height: Fit flow: Down padding: 14.0 spacing: 8.0
                    source := Label{text: "Input: calibrated physical leader"}
                    Label{text: "Leader serial port"}
                    leader_port := Setting{empty_text: "/dev/serial/by-id/…"}
                    Label{text: "Leader calibration"}
                    calibration_dir := Setting{empty_text: "Path to the saved calibration JSON"}
                    Label{text: "LeRobot Python executable"}
                    python := Setting{empty_text: "/path/to/lerobot/.venv/bin/python"}
                    Label{text: "Dataset and model output folder"}
                    output_root := Setting{empty_text: "Local output folder"}
                }
                status := Label{width: Fill max_lines: 2 text: ""}
                View{width: Fill height: Fit flow: Right spacing: 8.0
                    start := mod.widgets.ux.PrimaryButton{text: "Start Nexus capture"}
                    stop := mod.widgets.ux.StopButton{text: "Stop session"}
                    open_saved := mod.widgets.ux.ToolButton{visible: false text: "Open saved episode"}
                }
            }
        });
        Self {
            root: WidgetRef::script_from_value(vm, value),
            populated: false,
            light: None,
        }
    }
}

impl PluginPanel for NexusSetup {
    fn root(&self) -> WidgetRef {
        self.root.clone()
    }

    fn sync(&mut self, cx: &mut Cx, context: &PanelContext<'_>) {
        let light = frame::light_mode();
        if self.light != Some(light) {
            frame::apply_light(
                cx,
                &self.root,
                &[
                    (ids!(settings), Themed::Bg),
                    (ids!(start), Themed::Both),
                    (ids!(stop), Themed::Both),
                    (ids!(open_saved), Themed::Both),
                ],
                light,
            );
            self.light = Some(light);
        }
        let Some(state) = context.backend.nexus() else {
            self.root.button(cx, ids!(start)).set_enabled(cx, false);
            self.root.button(cx, ids!(stop)).set_enabled(cx, false);
            self.root
                .label(cx, ids!(status))
                .set_text(cx, "A local Studio backend is required");
            return;
        };
        if !self.populated {
            for (path, value) in [
                (ids!(python) as &[LiveId], &state.settings.python),
                (ids!(leader_port), &state.settings.leader_port),
                (ids!(calibration_dir), &state.settings.calibration_dir),
                (ids!(output_root), &state.settings.output_root),
            ] {
                self.root.text_input(cx, path).set_text(cx, value);
            }
            self.populated = true;
        }
        self.root
            .label(cx, ids!(status))
            .set_text(cx, &state.snapshot.message);
        self.root
            .button(cx, ids!(start))
            .set_enabled(cx, !state.busy);
        self.root
            .button(cx, ids!(stop))
            .set_visible(cx, state.kind == Some(NexusKind::Capture) && state.busy);
        self.root.button(cx, ids!(stop)).set_enabled(
            cx,
            state.kind == Some(NexusKind::Capture) && state.allows(&NexusIntent::Stop),
        );
        self.root.button(cx, ids!(open_saved)).set_visible(
            cx,
            !state.busy && state.allows(&NexusIntent::OpenSavedDataset),
        );
        if std::env::var("DOROBOT_NEXUS_SYNTHETIC").as_deref() == Ok("1") {
            self.root
                .label(cx, ids!(source))
                .set_text(cx, "Synthetic leader input");
        }
    }

    fn handle_actions(
        &mut self,
        cx: &mut Cx,
        actions: &Actions,
        backend: &mut dyn Backend,
    ) -> bool {
        if self.root.button(cx, ids!(start)).clicked(actions) {
            if let Some(state) = backend.nexus() {
                let mut settings = state.settings.clone();
                settings.python = self.root.text_input(cx, ids!(python)).text();
                settings.leader_port = self.root.text_input(cx, ids!(leader_port)).text();
                settings.calibration_dir = self.root.text_input(cx, ids!(calibration_dir)).text();
                settings.output_root = self.root.text_input(cx, ids!(output_root)).text();
                backend.dispatch(Intent::Nexus(NexusIntent::Configure(settings)));
                backend.dispatch(Intent::Nexus(NexusIntent::StartCapture));
            }
            return true;
        }
        if self.root.button(cx, ids!(stop)).clicked(actions) {
            backend.dispatch(Intent::Nexus(NexusIntent::Stop));
            return true;
        }
        if self.root.button(cx, ids!(open_saved)).clicked(actions) {
            backend.dispatch(Intent::Nexus(NexusIntent::OpenSavedDataset));
            return true;
        }
        false
    }
}
