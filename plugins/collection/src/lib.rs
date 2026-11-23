//! 数据采集插件：拥有真实采集界面和界面到业务服务的命令映射。
pub mod screen;
use dorobot_studio_core::{
    api::{nexus::{NexusIntent, NexusKind}, Backend, Intent, Screen},
    ui::{frame::{self, Themed}, nexus::NexusView},
};
use dorobot_studio_plugin::*;
use screen::RecordScreenWidgetRefExt;

pub struct CollectionPlugin;
impl StudioPlugin for CollectionPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: "dorobot.collection",
            title: "Data collection",
            api_version: API_VERSION,
            panels: &[PanelDescriptor {
                id: "page_record",
                title: "Record",
                legacy_screen: Some(Screen::Record),
            }],
        }
    }
    fn register(&self, vm: &mut ScriptVm) {
        screen::script_mod(vm);
    }
    fn create(&self, panel: &str, vm: &mut ScriptVm) -> Result<Box<dyn PluginPanel>, String> {
        if panel != "page_record" {
            return Err(format!("未知采集面板：{panel}"));
        }
        let value =
            script_eval!(vm, { use mod.prelude.widgets.* use mod.widgets.* RecordScreen{} });
        Ok(Box::new(CollectionPanel {
            root: WidgetRef::script_from_value(vm, value),
            nexus_view: NexusView::default(),
            light: None,
        }))
    }
    fn can_disable(&self, backend: &dyn Backend) -> Result<(), String> {
        let state = backend.record();
        if backend.nexus().is_some_and(|state| state.kind == Some(NexusKind::Capture) && state.busy)
            || state.recording
            || state.busy
            || state.last_take.is_some()
            || backend.workflow().pending()
        {
            Err("请先结束录制，并保存或丢弃当前片段。".into())
        } else {
            Ok(())
        }
    }
}
struct CollectionPanel {
    root: WidgetRef,
    nexus_view: NexusView,
    light: Option<f64>,
}
impl PluginPanel for CollectionPanel {
    fn root(&self) -> WidgetRef {
        self.root.clone()
    }
    fn visible_in(&self, context: &PanelContext<'_>) -> Option<bool> {
        let backend = context.backend;
        Some(backend.screen() == Screen::Record && (
            nexus_capture(backend) || (!backend.workflow().simulation
                && backend.hardware().nano.is_none()
                && backend.playback().live.is_none())
        ))
    }
    fn sync(&mut self, cx: &mut Cx, context: &PanelContext<'_>) {
        let backend = context.backend;
        let capture = nexus_capture(backend);
        let panel = self.root.widget(cx, ids!(nexus_capture));
        panel.set_visible(cx, capture);
        for path in [ids!(sbar), ids!(body), ids!(review), ids!(transport)] {
            self.root.widget(cx, path).set_visible(cx, !capture);
        }
        if capture {
            let light = frame::light_mode();
            if self.light != Some(light) {
                let mut root = self.root.clone();
                script_apply_eval!(cx, root, { draw_bg +: { light: #(light) } });
                let buttons: Vec<_> = nexus_actions(false)
                    .iter().map(|(path, _)| (*path, Themed::Both)).collect();
                frame::apply_light(cx, &panel, &buttons, light);
                self.light = Some(light);
            }
            let state = backend.nexus().unwrap();
            self.nexus_view.sync(cx, &panel.widget(cx, ids!(stage)), state, false);
            let snapshot = &state.snapshot;
            let prompt = match snapshot.phase.as_str() {
                "reset_required" => "Move the gripper clear of the tabletop, then reset the scene.",
                "ready" => "The cube is ready. Start an episode when you are ready to move.",
                "recording" => "Recording from Nexus. Place the cube on the plate, then finish the episode.",
                "review" => "Review the take, then save it or discard it and record again.",
                "saved" => "Reset for another take, or stop the session before training.",
                _ => &snapshot.message,
            };
            panel.label(cx, ids!(prompt)).set_text(cx, prompt);
            panel.label(cx, ids!(detail)).set_text(cx, if prompt == snapshot.message {
                ""
            } else {
                &snapshot.message
            });
            let input = match snapshot.source_mode.as_deref() {
                Some("synthetic") => "Synthetic leader input",
                Some("calibrated_leader") => "Calibrated physical leader",
                _ => "",
            };
            panel.label(cx, ids!(source)).set_text(cx, input);
            panel.label(cx, ids!(count)).set_text(cx, &format!("{} frames · {}", snapshot.frames, snapshot.phase));
            panel.button(cx, ids!(pause)).set_text(cx, if snapshot.phase == "paused" {"Resume"} else {"Pause"});
            for (path, intent) in nexus_actions(snapshot.phase == "paused") {
                let visible = match intent {
                    NexusIntent::Reset => matches!(snapshot.phase.as_str(), "reset_required" | "ready" | "paused" | "saved"),
                    NexusIntent::Pause => snapshot.phase == "ready",
                    NexusIntent::Step | NexusIntent::Resume => snapshot.phase == "paused",
                    NexusIntent::Record => snapshot.phase == "ready",
                    NexusIntent::Finish => snapshot.phase == "recording",
                    NexusIntent::Save | NexusIntent::Discard => snapshot.phase == "review",
                    NexusIntent::OpenSavedDataset => snapshot.dataset.is_some() && !matches!(snapshot.phase.as_str(), "recording" | "review" | "saving"),
                    _ => true,
                };
                let button = panel.button(cx, path);
                button.set_visible(cx, visible);
                button.set_enabled(cx, state.allows(&intent));
            }
        } else {
            self.root.as_record_screen().sync(cx, backend.record());
        }
    }
    fn handle_actions(
        &mut self,
        cx: &mut Cx,
        actions: &Actions,
        backend: &mut dyn Backend,
    ) -> bool {
        if nexus_capture(backend) {
            let Some(state) = backend.nexus() else { return false; };
            let panel = self.root.widget(cx, ids!(nexus_capture));
            for (path, intent) in nexus_actions(state.snapshot.phase == "paused") {
                if panel.button(cx, path).clicked(actions) {
                    backend.dispatch(Intent::Nexus(intent));
                    return true;
                }
            }
            return false;
        }
        for (path, intent) in [
            (
                ids!(transport.btn_record) as &[LiveId],
                if backend.record().recording {
                    Intent::RecordStop
                } else {
                    Intent::RecordStart
                },
            ),
            (ids!(transport.btn_save), Intent::SaveEpisode),
            (ids!(transport.btn_discard), Intent::DiscardLast),
            (ids!(transport.btn_rerecord), Intent::ReRecord),
        ] {
            if self.root.button(cx, path).clicked(actions) {
                backend.dispatch(intent);
                return true;
            }
        }
        false
    }
}

fn nexus_capture(backend: &dyn Backend) -> bool {
    backend.nexus().is_some_and(|state| state.kind == Some(NexusKind::Capture) && state.busy)
}

fn nexus_actions(paused: bool) -> [(&'static [LiveId], NexusIntent); 9] {
    [
        (ids!(reset), NexusIntent::Reset),
        (ids!(pause), if paused { NexusIntent::Resume } else { NexusIntent::Pause }),
        (ids!(step), NexusIntent::Step),
        (ids!(record), NexusIntent::Record),
        (ids!(finish), NexusIntent::Finish),
        (ids!(save), NexusIntent::Save),
        (ids!(discard), NexusIntent::Discard),
        (ids!(replay), NexusIntent::OpenSavedDataset),
        (ids!(stop), NexusIntent::Stop),
    ]
}
