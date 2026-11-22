use dorobot_studio_core::{
    api::{
        telemetry::{MirrorPhase, SourceId, TelemetryState},
        Backend, Intent,
    },
    data::{robot_model::RobotModelLoader, telemetry_model::TelemetryModel},
};
use dorobot_studio_plugin::*;
use makepad_urdf_player::robot_view::RobotViewWidgetRefExt;

pub struct LiveMirrorPanel {
    root: WidgetRef,
    model: Option<TelemetryModel>,
    desired_model: Option<String>,
    loaded_model: Option<String>,
    loader: RobotModelLoader,
    applied: Option<(SourceId, u64)>,
    model_error: Option<String>,
}

impl LiveMirrorPanel {
    pub fn create(vm: &mut ScriptVm) -> Self {
        let value = script_eval!(vm, {
            use mod.prelude.widgets.*
            use mod.widgets.*
            View {
                width: Fill height: Fill flow: Down spacing: 8
                padding: 10
                View {
                    width: Fill height: Fit flow: Right spacing: 10
                    align: Align{y: 0.5}
                    next_source := Button{text: "Select next source"}
                    stream := Label {text: "Searching for Zenoh robots"}
                }
                viewer := RobotView { width: Fill height: Fill }
                joints := Label {text: ""}
            }
        });
        Self {
            root: WidgetRef::script_from_value(vm, value),
            model: None,
            desired_model: None,
            loaded_model: None,
            loader: RobotModelLoader::default(),
            applied: None,
            model_error: None,
        }
    }

    fn update_viewer(&mut self, cx: &mut Cx, state: &TelemetryState) {
        let model_id = state.selected.as_ref().and_then(|selected| {
            state
                .sources
                .iter()
                .find(|source| &source.source == selected)
                .map(|source| source.model_id.as_str())
        });
        let viewer = self.root.robot_view(cx, ids!(viewer));
        // Keep the last mounted model while a selected robot disconnects.
        if model_id.is_some() || state.selected.is_none() {
            if self.desired_model.as_deref() != model_id {
                self.desired_model = model_id.map(str::to_owned);
                self.model = model_id.and_then(|id| {
                    TelemetryModel::resolve(id, &dorobot_studio_core::runtime::root())
                });
                self.loaded_model = None;
                self.applied = None;
                self.model_error = None;
                viewer.clear_robot(cx);
                self.loader.request(
                    self.model
                        .as_ref()
                        .map(|model| (model.urdf.clone(), model.assets.clone())),
                );
            }
        }
        if let Some(result) = self.loader.take_ready() {
            match result {
                Ok(robot) => {
                    if let Some(mut view) = viewer.borrow_mut() {
                        view.set_robot(cx, robot);
                        self.loaded_model.clone_from(&self.desired_model);
                    }
                }
                Err(error) => self.model_error = Some(error),
            }
        }
        let (Some(model), Some(selected), Some(model_id)) =
            (&self.model, &state.selected, &self.loaded_model)
        else {
            return;
        };
        let Some(frame) = state.render_frame(selected, model_id, std::time::Instant::now()) else {
            return;
        };
        if self
            .applied
            .as_ref()
            .is_some_and(|(source, sequence)| source == selected && *sequence == frame.sequence)
        {
            return;
        }
        let Some(mut view) = viewer.borrow_mut() else {
            return;
        };
        match model.viewer_angles(&frame.joints, &view.movable_joint_names()) {
            Ok(angles) => {
                view.set_joint_angles(cx, &angles);
                if ::log::log_enabled!(::log::Level::Trace) {
                    let age_ms = frame.capture_utc_ns.and_then(|stamp| {
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .ok()?
                            .as_nanos()
                            .checked_sub(u128::from(stamp))
                            .map(|ns| ns as f64 / 1_000_000.0)
                    });
                    ::log::trace!(
                        "live mirror applied {} sample {} capture_to_view_ms={age_ms:?}",
                        selected.robot_id,
                        frame.sequence
                    );
                }
                self.model_error = None;
            }
            Err(error) => self.model_error = Some(error.to_string()),
        }
        self.applied = Some((selected.clone(), frame.sequence));
    }
}

impl PluginPanel for LiveMirrorPanel {
    fn root(&self) -> WidgetRef {
        self.root.clone()
    }

    fn sync(&mut self, cx: &mut Cx, context: &PanelContext<'_>) {
        let Some(state) = context.backend.telemetry() else {
            return;
        };
        self.update_viewer(cx, state);
        let phase = match state.phase {
            MirrorPhase::Searching => "Searching for Zenoh robots",
            MirrorPhase::ChooseSource => "Select a robot session",
            MirrorPhase::Waiting => "Waiting for joints",
            MirrorPhase::Live if self.loader.loading() => "Loading robot model",
            MirrorPhase::Live => "Live",
            MirrorPhase::Stale => "Stale · holding last pose",
            MirrorPhase::Disconnected => "Disconnected · holding last pose",
            MirrorPhase::UnsupportedModel => "Unsupported model · showing joint data",
            MirrorPhase::Error => "Zenoh connection error",
        };
        let mut text = phase.to_owned();
        if let Some(source) = &state.selected {
            text.push_str(&format!(
                " · {} / {}",
                source.robot_id,
                &source.session_id[..8]
            ));
        }
        if let Some(pose) = &state.latest {
            text.push_str(&format!(
                " · {:.1} Hz · {} ms since update · {} missed",
                pose.receive_hz,
                pose.received_at.elapsed().as_millis(),
                pose.missed_samples
            ));
        }
        if let Some(error) = self.model_error.as_ref().or(state.error.as_ref()) {
            text.push_str(&format!(" · {error}"));
        }
        self.root.label(cx, ids!(stream)).set_text(cx, &text);
        self.root
            .button(cx, ids!(next_source))
            .set_visible(cx, state.can_select_source());
        let joint_text = if state.phase == MirrorPhase::UnsupportedModel {
            state
                .latest
                .as_ref()
                .map(|pose| {
                    pose.frame
                        .joints
                        .iter()
                        .map(|joint| {
                            format!(
                                "{}  {:.2} {}",
                                joint.name,
                                joint.position,
                                joint.unit.label()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        self.root.label(cx, ids!(joints)).set_text(cx, &joint_text);
    }

    fn handle_actions(
        &mut self,
        cx: &mut Cx,
        actions: &Actions,
        backend: &mut dyn Backend,
    ) -> bool {
        if !self.root.button(cx, ids!(next_source)).clicked(actions) {
            return false;
        }
        if let Some(state) = backend.telemetry() {
            let current = state
                .sources
                .iter()
                .position(|source| Some(&source.source) == state.selected.as_ref());
            let next = current.map_or(0, |index| (index + 1) % state.sources.len());
            if let Some(source) = state.sources.get(next) {
                backend.dispatch(Intent::SelectTelemetrySource(source.source.clone()));
            }
        }
        true
    }
}
