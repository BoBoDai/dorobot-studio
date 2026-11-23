use dorobot_studio_core::{
    api::{
        nexus::{NexusIntent, NexusKind, NexusState},
        Backend, Intent,
    },
    ui::{
        frame::{self, Themed},
        nexus::NexusView,
    },
    widgets::video_player::VideoPlayerWidgetRefExt,
};
use dorobot_studio_plugin::*;
use dorobot_ux::kit::SparkWidgetRefExt;
use std::{path::PathBuf, time::Instant};

pub struct PolicyPlugin;

impl StudioPlugin for PolicyPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: "dorobot.policy",
            title: "Training & inference",
            api_version: API_VERSION,
            panels: &[
                PanelDescriptor {
                    id: "nexus_train",
                    title: "Training",
                    legacy_screen: None,
                },
                PanelDescriptor {
                    id: "nexus_rollout",
                    title: "Inference & evaluation",
                    legacy_screen: None,
                },
            ],
        }
    }

    fn register(&self, _: &mut ScriptVm) {}

    fn create(&self, panel: &str, vm: &mut ScriptVm) -> Result<Box<dyn PluginPanel>, String> {
        match panel {
            "nexus_train" => Ok(Box::new(TrainingPanel::create(vm))),
            "nexus_rollout" => Ok(Box::new(RolloutPanel::create(vm))),
            _ => Err(format!("Unknown policy panel {panel}")),
        }
    }

    fn can_disable(&self, backend: &dyn Backend) -> Result<(), String> {
        if backend.nexus().is_some_and(|state| {
            state.busy && matches!(state.kind, Some(NexusKind::Train | NexusKind::Rollout))
        }) {
            Err("Stop the active training or inference job first.".into())
        } else {
            Ok(())
        }
    }
}

struct TrainingPanel {
    root: WidgetRef,
    source: Option<String>,
    steps: Option<String>,
    loss_history: Vec<(u64, f64)>,
    light: Option<f64>,
}

impl TrainingPanel {
    fn create(vm: &mut ScriptVm) -> Self {
        let value = script_eval!(vm, {
            use mod.prelude.widgets.*
            use mod.widgets.*
            View{width: Fill height: Fill flow: Down padding: 16.0 spacing: 12.0
                Label{text: "ACT training · Local host"
                    draw_text +: {text_style: mod.widgets.ux.TEXT_TITLE{}}}
                options := mod.widgets.ux.Card{width: Fill height: Fit flow: Down padding: 14.0 spacing: 8.0
                    Label{text: "LeRobot v3 dataset or dataset folder"}
                    View{width: Fill height: Fit flow: Right spacing: 8.0
                        dataset := TextInput{width: Fill height: 36 min_height: 36 empty_text: "Selected dataset path"}
                        current := mod.widgets.ux.ToolButton{text: "Use open dataset"}
                    }
                    View{width: Fill height: Fit flow: Right spacing: 8.0 align: Align{y: 0.5}
                        Label{text: "Steps"}
                        steps := TextInput{width: 100 height: 36 min_height: 36}
                        train := mod.widgets.ux.PrimaryButton{text: "Start training"}
                        stop := mod.widgets.ux.StopButton{text: "Cancel training"}
                    }
                }
                progress_card := mod.widgets.ux.Card{width: Fill height: Fit flow: Down padding: 14.0 spacing: 8.0
                    progress := Label{width: Fill text: "Training progress"}
                    loss := Spark{width: Fill height: 100}
                    status := Label{width: Fill max_lines: 3 text: ""}
                    checkpoint := Label{width: Fill max_lines: 2 text: ""}
                    run_checkpoint := mod.widgets.ux.PrimaryButton{text: "Run checkpoint in Nexus"}
                }
                log_card := mod.widgets.ux.Card{width: Fill height: Fill flow: Down padding: 12.0
                    Label{text: "Job log"}
                    logs := Label{width: Fill max_lines: 13 text: ""}
                }
            }
        });
        Self {
            root: WidgetRef::script_from_value(vm, value),
            source: None,
            steps: None,
            loss_history: Vec::new(),
            light: None,
        }
    }
}

impl PluginPanel for TrainingPanel {
    fn root(&self) -> WidgetRef {
        self.root.clone()
    }

    fn sync(&mut self, cx: &mut Cx, context: &PanelContext<'_>) {
        let light = frame::light_mode();
        let light_changed = self.light != Some(light);
        if light_changed {
            frame::apply_light(
                cx,
                &self.root,
                &[
                    (ids!(options), Themed::Bg),
                    (ids!(progress_card), Themed::Bg),
                    (ids!(log_card), Themed::Bg),
                    (ids!(current), Themed::Both),
                    (ids!(train), Themed::Both),
                    (ids!(stop), Themed::Both),
                    (ids!(run_checkpoint), Themed::Both),
                ],
                light,
            );
            self.light = Some(light);
        }
        let Some(state) = context.backend.nexus() else {
            self.root
                .label(cx, ids!(status))
                .set_text(cx, "A local Studio backend is required");
            for path in [ids!(train), ids!(stop), ids!(current), ids!(run_checkpoint)] {
                self.root.button(cx, path).set_enabled(cx, false);
            }
            return;
        };
        sync_input(
            &self.root,
            cx,
            ids!(dataset),
            &mut self.source,
            &state.settings.dataset,
        );
        sync_input(
            &self.root,
            cx,
            ids!(steps),
            &mut self.steps,
            &state.settings.train_steps,
        );
        let snapshot = &state.snapshot;
        let phase = if state.kind == Some(NexusKind::Train) {
            snapshot.phase.as_str()
        } else {
            "idle"
        };
        let loss = state
            .loss_history
            .last()
            .map(|(_, loss)| format!(" · loss {loss:.3}"))
            .unwrap_or_default();
        let progress = if state.kind == Some(NexusKind::Train) {
            format!(
                "{} · {}/{} steps{}",
                phase, snapshot.steps, snapshot.total_steps, loss
            )
        } else {
            "Select a dataset to train ACT".into()
        };
        self.root.label(cx, ids!(progress)).set_text(cx, &progress);
        if light_changed || self.loss_history != state.loss_history {
            self.loss_history.clone_from(&state.loss_history);
            let values: Vec<_> = self.loss_history.iter().map(|(_, value)| *value).collect();
            self.root
                .spark(cx, ids!(loss))
                .set(cx, &values, None, 3, light as f32);
        }
        self.root
            .label(cx, ids!(status))
            .set_text(cx, &snapshot.message);
        self.root
            .label(cx, ids!(checkpoint))
            .set_text(cx, &format!("Checkpoint  {}", state.settings.checkpoint));
        self.root
            .label(cx, ids!(logs))
            .set_text(cx, &state.recent_log.join("\n"));
        self.root
            .button(cx, ids!(train))
            .set_enabled(cx, !state.busy);
        self.root
            .button(cx, ids!(stop))
            .set_visible(cx, state.kind == Some(NexusKind::Train) && state.busy);
        self.root.button(cx, ids!(stop)).set_enabled(
            cx,
            state.kind == Some(NexusKind::Train) && state.allows(&NexusIntent::Stop),
        );
        self.root
            .button(cx, ids!(current))
            .set_enabled(cx, !state.busy && context.backend.dataset_id().is_some());
        self.root
            .button(cx, ids!(run_checkpoint))
            .set_visible(cx, !state.settings.checkpoint.is_empty());
        self.root
            .button(cx, ids!(run_checkpoint))
            .set_enabled(cx, !state.settings.checkpoint.is_empty() && !state.busy);
    }

    fn handle_actions(
        &mut self,
        cx: &mut Cx,
        actions: &Actions,
        backend: &mut dyn Backend,
    ) -> bool {
        if self.root.button(cx, ids!(train)).clicked(actions) {
            if let Some(state) = backend.nexus() {
                let mut settings = state.settings.clone();
                settings.dataset = self.root.text_input(cx, ids!(dataset)).text();
                settings.train_steps = self.root.text_input(cx, ids!(steps)).text();
                backend.dispatch(Intent::Nexus(NexusIntent::Configure(settings)));
                backend.dispatch(Intent::Nexus(NexusIntent::Train));
            }
            return true;
        }
        for (path, intent) in [
            (ids!(current) as &[LiveId], NexusIntent::UseOpenDataset),
            (ids!(stop), NexusIntent::Stop),
            (ids!(run_checkpoint), NexusIntent::Run),
        ] {
            if self.root.button(cx, path).clicked(actions) {
                backend.dispatch(Intent::Nexus(intent));
                return true;
            }
        }
        false
    }
}

struct RolloutPanel {
    root: WidgetRef,
    view: NexusView,
    checkpoint: Option<String>,
    steps: Option<String>,
    video: Option<PathBuf>,
    playback_start: Option<Instant>,
    playback_frame: u64,
    light: Option<f64>,
}

impl RolloutPanel {
    fn create(vm: &mut ScriptVm) -> Self {
        let value = script_eval!(vm, {
            use mod.prelude.widgets.*
            use mod.widgets.*
            View{width: Fill height: Fill flow: Down padding: 16.0 spacing: 10.0
                Label{text: "ACT · Nexus simulation · SO-101 · Local host"
                    draw_text +: {text_style: mod.widgets.ux.TEXT_TITLE{}}}
                View{width: Fill height: Fit flow: Right spacing: 8.0 align: Align{y: 0.5}
                    checkpoint := TextInput{width: Fill height: 36 min_height: 36 empty_text: "ACT checkpoint folder"}
                    Label{text: "Steps"}
                    steps := TextInput{width: 80 height: 36 min_height: 36}
                    run := mod.widgets.ux.PrimaryButton{text: "Run policy"}
                    stop := mod.widgets.ux.StopButton{text: "Stop rollout"}
                }
                Label{text: "Observation: front RGB and six joints · Action: joint targets and gripper"}
                stage := NexusStage{}
                result_card := mod.widgets.ux.Card{width: Fill height: Fit flow: Down padding: 10.0 spacing: 6.0
                    status := Label{width: Fill max_lines: 2 text: ""}
                    result := Label{width: Fill text: ""}
                    artifact := Label{width: Fill max_lines: 1 text_overflow: Ellipsis text: ""}
                    replay := mod.widgets.ux.ToolButton{text: "Replay rollout"}
                    video_error := Label{width: Fill max_lines: 2 text: ""}
                }
            }
        });
        Self {
            root: WidgetRef::script_from_value(vm, value),
            view: NexusView::default(),
            checkpoint: None,
            steps: None,
            video: None,
            playback_start: None,
            playback_frame: 0,
            light: None,
        }
    }

    fn update_replay(&mut self, cx: &mut Cx, state: &NexusState, stage: &WidgetRef) {
        if state.busy && state.kind == Some(NexusKind::Rollout) {
            self.video = None;
            self.playback_start = None;
        }
        self.view.sync(cx, stage, state, self.video.is_some());
        if let Some(start) = self.playback_start {
            let frame = (start.elapsed().as_secs_f64() * 20.) as u64;
            if frame >= state.snapshot.frames {
                self.playback_start = None;
            } else if frame != self.playback_frame {
                let camera = stage.video_player(cx, ids!(camera));
                camera.set_frame_info(cx, frame, state.snapshot.frames);
                camera.show_frame_at_time(cx, frame as f64 / 20.);
                self.playback_frame = frame;
            }
        }
    }
}

impl PluginPanel for RolloutPanel {
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
                    (ids!(result_card), Themed::Bg),
                    (ids!(run), Themed::Both),
                    (ids!(stop), Themed::Both),
                    (ids!(replay), Themed::Both),
                ],
                light,
            );
            self.light = Some(light);
        }
        let Some(state) = context.backend.nexus() else {
            self.root
                .label(cx, ids!(status))
                .set_text(cx, "A local Studio backend is required");
            for path in [ids!(run), ids!(stop), ids!(replay)] {
                self.root.button(cx, path).set_enabled(cx, false);
            }
            return;
        };
        sync_input(
            &self.root,
            cx,
            ids!(checkpoint),
            &mut self.checkpoint,
            &state.settings.checkpoint,
        );
        sync_input(
            &self.root,
            cx,
            ids!(steps),
            &mut self.steps,
            &state.settings.rollout_steps,
        );
        let stage = self.root.widget(cx, ids!(stage));
        stage.set_visible(cx, state.kind == Some(NexusKind::Rollout));
        if state.kind == Some(NexusKind::Rollout) {
            self.update_replay(cx, state, &stage);
            let snapshot = &state.snapshot;
            self.root.label(cx, ids!(status)).set_text(
                cx,
                &format!("Run {} · {}", snapshot.phase, snapshot.message),
            );
            let result = match snapshot.task_success {
                Some(true) => "Task result: cube placed on plate",
                Some(false) => "Task result: placement not completed",
                None => "Task result: pending",
            };
            self.root.label(cx, ids!(result)).set_text(cx, result);
            self.root.label(cx, ids!(artifact)).set_text(
                cx,
                &snapshot
                    .video
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
            );
        } else {
            self.root
                .label(cx, ids!(status))
                .set_text(cx, "Choose a compatible ACT checkpoint.");
            self.root.label(cx, ids!(result)).set_text(cx, "");
            self.root.label(cx, ids!(artifact)).set_text(cx, "");
        }
        self.root.button(cx, ids!(run)).set_enabled(cx, !state.busy);
        self.root
            .button(cx, ids!(stop))
            .set_visible(cx, state.kind == Some(NexusKind::Rollout) && state.busy);
        self.root.button(cx, ids!(stop)).set_enabled(
            cx,
            state.kind == Some(NexusKind::Rollout) && state.allows(&NexusIntent::Stop),
        );
        self.root.button(cx, ids!(replay)).set_enabled(
            cx,
            state.kind == Some(NexusKind::Rollout) && state.snapshot.video.is_some() && !state.busy,
        );
        self.root.button(cx, ids!(replay)).set_visible(
            cx,
            state.kind == Some(NexusKind::Rollout) && state.snapshot.video.is_some() && !state.busy,
        );
    }

    fn handle_actions(
        &mut self,
        cx: &mut Cx,
        actions: &Actions,
        backend: &mut dyn Backend,
    ) -> bool {
        if self.root.button(cx, ids!(run)).clicked(actions) {
            if let Some(state) = backend.nexus() {
                let mut settings = state.settings.clone();
                settings.checkpoint = self.root.text_input(cx, ids!(checkpoint)).text();
                settings.rollout_steps = self.root.text_input(cx, ids!(steps)).text();
                backend.dispatch(Intent::Nexus(NexusIntent::Configure(settings)));
                backend.dispatch(Intent::Nexus(NexusIntent::Run));
            }
            return true;
        }
        if self.root.button(cx, ids!(stop)).clicked(actions) {
            backend.dispatch(Intent::Nexus(NexusIntent::Stop));
            return true;
        }
        if self.root.button(cx, ids!(replay)).clicked(actions) {
            if let Some(snapshot) = backend.nexus().map(|state| &state.snapshot) {
                let Some(path) = &snapshot.video else {
                    return false;
                };
                let player = self.root.video_player(cx, ids!(stage.camera));
                player.set_frame_info(cx, 0, snapshot.frames);
                match player.load_video(cx, &path.to_string_lossy()) {
                    Ok(()) => {
                        self.video = Some(path.clone());
                        self.playback_start = Some(Instant::now());
                        self.playback_frame = 0;
                        self.root.label(cx, ids!(video_error)).set_text(cx, "");
                    }
                    Err(error) => self.root.label(cx, ids!(video_error)).set_text(cx, &error),
                }
            }
            return true;
        }
        false
    }
}

fn sync_input(
    root: &WidgetRef,
    cx: &mut Cx,
    path: &[LiveId],
    previous: &mut Option<String>,
    text: &str,
) {
    if previous.as_deref() != Some(text) {
        root.text_input(cx, path).set_text(cx, text);
        *previous = Some(text.to_owned());
    }
}
