//! 机器人设置与实时操作插件。
pub mod screen;
mod live_mirror;
mod nexus_setup;
use dorobot_studio_core::api::{Backend, Screen};
use dorobot_studio_core::ui::play::PlayScreenWidgetRefExt;
use dorobot_studio_plugin::*;
use screen::HardwareScreenWidgetRefExt;

pub struct ControlPlugin;
impl StudioPlugin for ControlPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: "dorobot.control",
            title: "Robot control",
            api_version: API_VERSION,
            panels: &[
                PanelDescriptor {
                    id: "page_hardware",
                    title: "Robot setup",
                    legacy_screen: Some(Screen::Hardware),
                },
                PanelDescriptor {
                    id: "hardware_teleop",
                    title: "Operate",
                    legacy_screen: Some(Screen::Live),
                },
                PanelDescriptor {
                    id: "nexus_setup",
                    title: "SO-101 Nexus",
                    legacy_screen: None,
                },
                PanelDescriptor {
                    id: "hardware_live_mirror",
                    title: "Live mirror",
                    legacy_screen: None,
                },
            ],
        }
    }
    fn register(&self, vm: &mut ScriptVm) {
        screen::script_mod(vm);
    }
    fn create(&self, panel: &str, vm: &mut ScriptVm) -> Result<Box<dyn PluginPanel>, String> {
        match panel {
            "page_hardware" => {
                let value = script_eval!(vm, { use mod.prelude.widgets.* use mod.widgets.* HardwareScreen{} });
                Ok(Box::new(HardwarePanel {
                    root: WidgetRef::script_from_value(vm, value),
                }))
            }
            "hardware_teleop" => {
                let value = script_eval!(vm, {
                    use mod.prelude.widgets.* use mod.widgets.*
                    PlayScreen{
                        toolbar +: {align: Align{y: 0.5}
                            camera_hint := Label{width: Fit text: "Click camera/map · Drag 3D"
                                draw_text +: {color: mod.theme.color_text_disabled text_style: mod.widgets.ux.TEXT_META{}}}
                        }
                        upper +: {grid +: {persistence_key: "hardware-camera-layout.json"}}
                    }
                });
                Ok(Box::new(OperatePanel {
                    root: WidgetRef::script_from_value(vm, value),
                }))
            }
            "nexus_setup" => Ok(Box::new(nexus_setup::NexusSetup::create(vm))),
            "hardware_live_mirror" => Ok(Box::new(live_mirror::LiveMirrorPanel::create(vm))),
            _ => Err(format!("未知控制面板：{panel}")),
        }
    }
    fn can_disable(&self, backend: &dyn Backend) -> Result<(), String> {
        if backend.workflow().active
            || backend.workflow().pending()
            || backend.record().recording
            || backend.record().busy
            || backend.hardware().nano.as_ref().is_some_and(|n| n.active)
        {
            Err("请先结束当前控制或采集会话。".into())
        } else {
            Ok(())
        }
    }
}
struct HardwarePanel {
    root: WidgetRef,
}
impl PluginPanel for HardwarePanel {
    fn root(&self) -> WidgetRef {
        self.root.clone()
    }
    fn visible_in(&self, context: &PanelContext<'_>) -> Option<bool> {
        Some(
            context.backend.screen() == Screen::Hardware
                || (context.backend.screen() == Screen::Live
                    && context.backend.hardware().nano.is_some()),
        )
    }
    fn sync(&mut self, cx: &mut Cx, context: &PanelContext<'_>) {
        let mut hardware = context.backend.hardware().clone();
        if context.backend.workflow().simulation {
            hardware.nano = None;
            let assets = dorobot_studio_core::runtime::root().join("data/adora_nano");
            hardware.robot_urdf = Some((assets.join("adora_nano.urdf"), assets));
            hardware.can_advance = false;
            hardware.joints.clear();
            hardware.steps.clear();
            hardware.instruction.clear();
        }
        if !context.backend.workflow().simulation && hardware.nano.is_none() {
            if context.rig_editing {
                hardware.can_advance = false;
                hardware.instruction = "Save this rig or cancel changes before calibrating.".into();
            }
            use dorobot_studio_core::api::{StepState, WizardStep};
            let profile = !context.backend.record().profile_label.is_empty();
            let calibrated = context.backend.workflow().calibrated;
            let cameras = !context.backend.record().previews.is_empty();
            hardware.steps = [
                ("Profile", profile),
                ("Devices", calibrated),
                ("Calibrate", calibrated),
                ("Cameras", cameras),
                (
                    "Record",
                    context.backend.record().recording
                        || context.backend.record().last_take.is_some(),
                ),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (title, done))| WizardStep {
                title: title.into(),
                state: if done {
                    StepState::Done
                } else if (i == 0 && !profile)
                    || (i == 2 && profile && !calibrated)
                    || (i == 3 && calibrated && !cameras)
                {
                    StepState::Active
                } else {
                    StepState::Pending
                },
            })
            .collect();
        }
        self.root.as_hardware_screen().sync(cx, &hardware);
        let sim_setup = context.backend.workflow().simulation;
        self.root
            .widget(cx, ids!(supported_robots))
            .set_visible(cx, false);
        if !sim_setup && hardware.nano.is_none() {
            self.root
                .button(
                    cx,
                    ids!(content.camera_a_panel.content.calib.actions.btn_continue),
                )
                .set_text(
                    cx,
                    if context.backend.record().busy {
                        "Continue"
                    } else if context.backend.workflow().calibrated {
                        "Calibrate again"
                    } else {
                        "Start calibration"
                    },
                );
        }
        self.root.as_hardware_screen().set_inspector_available(
            cx,
            !sim_setup
                && (hardware.nano.is_some() || !context.backend.record().profile_label.is_empty()),
        );
        self.root
            .widget(cx, ids!(layout_tools.hardware_layout_hint))
            .set_visible(cx, context.show_help);
        self.root
            .widget(cx, ids!(layout_tools.hardware_reset))
            .set_visible(cx, context.show_details);
        self.root
            .widget(
                cx,
                ids!(
                    content
                        .camera_a_panel
                        .content
                        .calib
                        .nano_controls
                        .wheel_commands
                ),
            )
            .set_visible(cx, context.show_details);
        if sim_setup {
            self.root.widget(cx, ids!(stepper)).set_visible(cx, false);
            self.root
                .widget(cx, ids!(content.camera_a_panel.content.calib.actions))
                .set_visible(cx, false);
            self.root
                .label(cx, ids!(content.camera_a_panel.head.title))
                .set_text(cx, "Simulation readiness");
        }
    }
    fn handle_actions(
        &mut self,
        cx: &mut Cx,
        actions: &Actions,
        backend: &mut dyn Backend,
    ) -> bool {
        if let Some(intent) = self.root.as_hardware_screen().wizard_intent(cx, actions) {
            backend.dispatch(intent);
            true
        } else {
            false
        }
    }
}
struct OperatePanel {
    root: WidgetRef,
}
impl PluginPanel for OperatePanel {
    fn root(&self) -> WidgetRef {
        self.root.clone()
    }
    fn visible_in(&self, context: &PanelContext<'_>) -> Option<bool> {
        Some(
            context.live_controls
                && matches!(context.backend.screen(), Screen::Live | Screen::Record)
                && context.backend.playback().live.is_some()
                && context.backend.hardware().nano.is_none(),
        )
    }
    fn sync(&mut self, cx: &mut Cx, context: &PanelContext<'_>) {
        self.root
            .as_play_screen()
            .sync(cx, context.backend.playback());
    }
    fn handle_actions(
        &mut self,
        _cx: &mut Cx,
        _actions: &Actions,
        _backend: &mut dyn Backend,
    ) -> bool {
        false
    }
}
