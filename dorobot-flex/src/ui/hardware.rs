//! Hardware setup & calibration wizard.
//!
//! Mirrors `docs/ux/ux-02-hardware.png`. The left panel is where
//! `makepad_urdf_player::RobotView` mounts once a rig profile supplies a URDF;
//! until then it renders an empty viewport so the screen is still verifiable.

use super::flex_layout::FlexPanelsWidgetExt;
use makepad_widgets::*;

use crate::api::{HardwareState, Intent, JointProgress, StepState};
use crate::ui::frame::{apply_light, apply_light_in, Themed};

script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*
    use mod.widgets.ux.*

    // Stepper node: ring + number, or a filled check once the step is done.
    let StepDot = View{
        width: Fit height: 26
        flow: Right
        align: Align{y: 0.5}
        spacing: 9.0

        dot := View{
            width: 26 height: 26
            flow: Overlay
            align: Align{x: 0.5 y: 0.5}
            show_bg: true
            draw_bg +: {
                // 0 pending, 1 active, 2 done
                state: instance(0.0)
                light: instance(0.0)
                pixel: fn() {
                    let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                    let active = step(0.5, self.state) * (1.0 - step(1.5, self.state))
                    let done = step(1.5, self.state)
                    let idle_ring = mix(#x38383A, #xD1D1D6, self.light)
                    let ring = mix(idle_ring, #x007AFF, active + done)
                    let base = mix(#x1C1C1E, #xFFFFFF, self.light)
                    let fill = mix(base, #x007AFF, active)
                    sdf.circle(13.0, 13.0, 11.5)
                    sdf.fill_keep(fill)
                    sdf.stroke(ring, 1.6)
                    // checkmark only in the done state
                    sdf.move_to(7.5, 13.2)
                    sdf.line_to(11.4, 17.0)
                    sdf.line_to(18.5, 9.4)
                    sdf.stroke(mix(#x00000000, #x248A3D, done), 2.0)
                    return sdf.result
                }
            }
            num := Label{
                padding: 0.0 margin: 0.0
                text: "1"
                draw_text +: {
                    temp_y_shift: #(if cfg!(target_os = "macos") { -0.10f64 } else { 0.0f64 })
                    state: instance(0.0)
                    light: instance(0.0)
                    text_style: mod.widgets.ux.TEXT_CHIP{}
                    get_color: fn() {
                        let active = step(0.5, self.state) * (1.0 - step(1.5, self.state))
                        let idle = mix(#xD1D1D6, #x636366, self.light)
                        return mix(idle, #xFFFFFF, active)
                    }
                }
            }
        }
        caption := Label{
            padding: 0.0 margin: 0.0
            text: "Step"
            draw_text +: {
                temp_y_shift: #(if cfg!(target_os = "macos") { -0.10f64 } else { 0.0f64 })
                state: instance(0.0)
                light: instance(0.0)
                text_style: mod.widgets.ux.TEXT_BODY{}
                get_color: fn() {
                    let active = step(0.5, self.state) * (1.0 - step(1.5, self.state))
                    let done = step(1.5, self.state)
                    let pending = mix(#xAEAEB2, #x636366, self.light)
                    let fin = mix(#xD1D1D6, #x38383A, self.light)
                    let acc = mix(#x64B5FF, #x0060CC, self.light)
                    return mix(mix(pending, fin, done), acc, active)
                }
            }
        }
    }

    let StepLine = View{
        width: Fill height: 1
        margin: Inset{left: 6. right: 6.}
        show_bg: true
        draw_bg +: {
            done: instance(0.0)
            light: instance(0.0)
            pixel: fn() {
                let idle = mix(#x38383A, #xE5E5EA, self.light)
                return mix(idle, #x007AFF, self.done)
            }
        }
    }

    // One calibration row: status pip, index, name, swept-range bar, readout.
    let JointRow = View{
        width: Fill height: 52
        flow: Right
        align: Align{y: 0.5}
        spacing: 6.0
        padding: Inset{left: 10. right: 10.}
        show_bg: true
        draw_bg +: {
            light: instance(0.0)
            pixel: fn() {
                // hairline separator on the bottom edge
                let base = mix(#x2C2C2E, #xFFFFFF, self.light)
                let sep = mix(#x38383A, #xF2F2F7, self.light)
                let t = step(self.rect_size.y - 1.0, self.pos.y * self.rect_size.y)
                return mix(base, sep, t)
            }
        }

        pip := View{
            width: 22 height: 22
            show_bg: true
            draw_bg +: {
                done: instance(0.0)
                light: instance(0.0)
                pixel: fn() {
                    let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                    let empty = mix(#x1C1C1E, #xFFFFFF, self.light)
                    let ring = mix(mix(#xAEAEB2, #x636366, self.light), #x248A3D, self.done)
                    sdf.circle(11.0, 11.0, 9.0)
                    sdf.fill_keep(mix(empty, #x30D158, self.done))
                    sdf.stroke(ring, 1.4)
                    sdf.move_to(6.5, 11.2)
                    sdf.line_to(9.7, 14.4)
                    sdf.line_to(15.4, 7.8)
                    sdf.stroke(mix(#x00000000, #x248A3D, self.done), 1.8)
                    return sdf.result
                }
            }
        }
        idx := Label{
            text: "1"
            draw_text +: {
                light: instance(0.0)
                text_style: mod.widgets.ux.TEXT_META{}
                get_color: fn() { return mix(#xAEAEB2, #x636366, self.light) }
            }
        }
        jname := Label{
            width: 100 max_lines: 1 text_overflow: Ellipsis
            text: "joint"
            draw_text +: {
                light: instance(0.0)
                text_style: mod.widgets.ux.TEXT_BODY{}
                get_color: fn() { return mix(#xD1D1D6, #x38383A, self.light) }
            }
        }
        bar := View{
            width: Fill height: 12
            show_bg: true
            draw_bg +: {
                frac: instance(0.0)
                done: instance(0.0)
                light: instance(0.0)
                pixel: fn() {
                    let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                    sdf.box(0.0, 3.0, self.rect_size.x, 6.0, 3.0)
                    sdf.fill(mix(#x38383A, #xE5E5EA, self.light))
                    // swept portion, centred like a range sweep
                    let w = self.rect_size.x * self.frac
                    let x0 = (self.rect_size.x - w) * 0.5
                    sdf.box(x0, 3.0, w, 6.0, 3.0)
                    sdf.fill(mix(#x007AFF, #x248A3D, self.done))
                    return sdf.result
                }
            }
        }
        readout := Label{
            width: 128 max_lines: 1 text_overflow: Ellipsis
            text: ""
            draw_text +: {
                warn: instance(0.0)
                light: instance(0.0)
                text_style: mod.widgets.ux.TEXT_META{}
                get_color: fn() {
                    let normal = mix(#xD1D1D6, #x636366, self.light)
                    let alert = mix(#xFF9F0A, #x996300, self.light)
                    return mix(normal, alert, self.warn)
                }
            }
        }
    }

    mod.widgets.HardwareScreenBase = #(HardwareScreen::register_widget(vm))
    mod.widgets.HardwareScreen = set_type_default() do mod.widgets.HardwareScreenBase{
        width: Fill
        height: Fill
        flow: Down
        spacing: 6.0
        padding: 6.0
        show_bg: true
        draw_bg +: {
            light: instance(0.0)
            pixel: fn() { return mix(#x111114, #xF2F2F7, self.light) }
        }

        supported_robots := View{width: Fill height: 32 flow: Right spacing: 6.0 align: Align{y: 0.5}
            simulation_select := mod.widgets.ux.ToolButton{text: "Nano simulation"}
            nano_select := mod.widgets.ux.ToolButton{text: "Adora Nano · Feetech base"}
            rig_select := mod.widgets.ux.ToolButton{text: "LeRobot rig"}
        }

        stepper := View{
            width: Fill height: 32
            flow: Right
            align: Align{y: 0.5}
            padding: Inset{left: 10. right: 10.}
            step_0 := StepDot{}
            line_0 := StepLine{}
            step_1 := StepDot{}
            line_1 := StepLine{}
            step_2 := StepDot{}
            line_2 := StepLine{}
            step_3 := StepDot{}
            line_3 := StepLine{}
            step_4 := StepDot{}
        }

        // Sub-windows are real panels, as on the Play screen: PanelGrid
        // owns the layout, so they drag, resize, maximise and persist.
        // Each Card keeps its body verbatim inside the slot — its own
        // walk and its children's are what lay the contents out.
        layout_tools := View{width: Fill height: 32 flow: Right spacing: 10.0 align: Align{y: 0.5}
            hardware_reset := mod.widgets.ux.QuietButton{text: "Restore panels"}
            hardware_fit := mod.widgets.ux.QuietButton{text: "Fit view"}
            hardware_layout_hint := Label{visible: false width: Fill height: Fit padding: 0.0 text: "Drag panel titles to arrange · drag edges to resize"
                draw_text +: {color: mod.theme.color_text text_style: mod.widgets.ux.TEXT_META{}}}
        }
        content := FlexPanels{panel_count: 2 persistence_key: "hardware-layout.json"
            robot_panel := StudioPanel{head +: {title +: {text: "Live mirror"}}
                content := View{width: Fill height: Fill mirror := View{
                        width: Fill height: Fill
                        flow: Down
                        // The real renderer, which brings its own studio gradient with
                        // it. This was a shader drawing a perspective grid with
                        // "RobotView mounts here" written across it.
                        viewport := RobotView{
                        keyboard_controls: false width: Fill height: Fill
                        material_roughness: 0.68 material_metallic: 0.02
                        sky_horizon: mod.theme.color_bg_app sky_zenith: mod.theme.color_bg_app
                        ground_color: mod.theme.color_bg_container ground2_color: mod.theme.color_bg_container
                        grid_color: mod.theme.color_text_disabled }
                    }}
            }
            camera_a_panel := StudioPanel{head +: {title +: {text: "Joint calibration"}}
                content := View{width: Fill height: Fill calib := ScrollYView{
                        width: Fill height: Fill
                        flow: Down

                        nano_controls := View{visible: false width: Fill height: Fit flow: Down spacing: 10.0 padding: 12.0
                            driver_state := Label{width: Fill height: Fit padding: 0.0
                                draw_text +: {color: mod.theme.color_text text_style: mod.widgets.ux.TEXT_TITLE{}}}
                            serial_row := View{width: Fill height: 32 flow: Right spacing: 6.0
                                port := TextInput{width: Fill height: 32 min_height: 32 margin: 0.0
                                    empty_text: "USB serial port"}
                                scan := mod.widgets.ux.ToolButton{text: "Find port"}
                            }
                            connection_row := View{width: Fill height: Fit flow: Flow.Right{wrap: true} spacing: 6.0
                                connect := mod.widgets.ux.PrimaryButton{text: "Connect"}
                                test := mod.widgets.ux.QuietButton{text: "Bus test"}
                                stop := mod.widgets.ux.StopButton{text: "Disconnect"}
                            }
                            wheel_commands := Label{width: Fill height: Fit padding: 0.0
                                draw_text +: {color: mod.theme.color_text text_style: mod.widgets.ux.TEXT_META{}}}
                            help := Label{width: Fill height: Fit padding: 0.0
                                draw_text +: {color: mod.theme.color_text text_style: mod.widgets.ux.TEXT_META{}}}
                        }

                        banner := ScrollYView{
                            width: Fill height: Fill
                            margin: Inset{left: 6. right: 6. top: 6. bottom: 0.}
                            padding: 10.0
                            flow: Right
                            align: Align{y: 0.0}
                            spacing: 16.0
                            show_bg: true
                            draw_bg +: {
                                light: instance(0.0)
                                border_size: 1.0
                                border_radius: 0.0
                                pixel: fn() {
                                    let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                                    sdf.box(0.5, 0.5, self.rect_size.x - 1.0, self.rect_size.y - 1.0, 0.5)
                                    sdf.fill_keep(mix(#x2C2C2E, #xFFFFFF, self.light))
                                    sdf.stroke(mix(#x38383A, #xE5E5EA, self.light), 1.0)
                                    return sdf.result
                                }
                            }
                            ring := View{
                                width: 54 height: 54
                                flow: Overlay
                                align: Align{x: 0.5 y: 0.5}
                                show_bg: true
                                draw_bg +: {
                                    frac: instance(0.0)
                                    light: instance(0.0)
                                    pixel: fn() {
                                        let p = self.pos * self.rect_size - vec2(27.0, 27.0)
                                        let d = length(p)
                                        let band = (1.0 - step(24.0, d)) * step(19.0, d)
                                        // angle from 12 o'clock, clockwise, 0..1
                                        let a = fract(atan2(p.x, -p.y) / 6.2831853 + 1.0)
                                        let on = 1.0 - step(self.frac, a)
                                        let track = mix(mix(#x38383A, #xE5E5EA, self.light), #x248A3D, on)
                                        return mix(#x00000000, track, band)
                                    }
                                }
                                ring_text := Label{
                                    padding: 0.0 margin: 0.0
                                    text: "0/0"
                                    draw_text +: {
                                        temp_y_shift: #(if cfg!(target_os = "macos") { -0.10f64 } else { 0.0f64 })
                                        light: instance(0.0)
                                        text_style: mod.widgets.ux.TEXT_TITLE{}
                                        get_color: fn() { return mix(#xE5E5EA, #x38383A, self.light) }
                                    }
                                }
                            }
                            btext := View{
                                width: Fill height: Fit flow: Down spacing: 5.0
                                instruction := Label{
                                    width: Fill padding: 0.0
                                    text: "Move every joint through its full range"
                                    draw_text +: {
                                        light: instance(0.0)
                                        text_style: mod.widgets.ux.TEXT_BODY{}
                                        get_color: fn() { return mix(#xE5E5EA, #x38383A, self.light) }
                                    }
                                }
                                subline := Label{
                                    width: Fill padding: 0.0
                                    text: "0/0 joints complete"
                                    draw_text +: { color: mod.theme.color_text text_style: mod.widgets.ux.TEXT_META{} }
                                }
                            }
                        }

                        rows := View{
                            width: Fill height: Fit
                            flow: Down
                            margin: Inset{left: 12. right: 12. top: 12.}
                            joint_0 := JointRow{}
                            joint_1 := JointRow{}
                            joint_2 := JointRow{}
                            joint_3 := JointRow{}
                            joint_4 := JointRow{}
                            joint_5 := JointRow{}
                        }

                        actions := View{
                            width: Fill height: Fit
                            flow: Flow.Right{wrap: true}
                            align: Align{y: 0.5}
                            spacing: 8.0
                            padding: Inset{left: 12. right: 12. bottom: 14.}
                            btn_restart := Button{
                                text: "Restart step" width: 120 height: 32 min_height: 32 margin: 0.0
                                padding: Inset{left: 12. right: 12. top: 0. bottom: 0.}
                                draw_bg +: {
                                    light: instance(0.0)
                                    pixel: fn() {
                                        let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                                        sdf.box(0.5, 0.5, self.rect_size.x - 1.0, self.rect_size.y - 1.0, 4.0)
                                        let idle = mix(#x2C2C2E, #xFFFFFF, self.light)
                                        let hov = mix(#x38383A, #xF2F2F7, self.light)
                                        sdf.fill_keep(mix(idle, hov, self.hover))
                                        sdf.stroke(mix(#x38383A, #xD1D1D6, self.light), 1.0)
                                        return sdf.result
                                    }
                                }
                                draw_text +: {
                                    light: instance(0.0)
                                    text_style: mod.widgets.ux.TEXT_BODY{}
                                    get_color: fn() { return mix(#xD1D1D6, #x38383A, self.light) }
                                }
                            }
                            btn_continue := Button{
                                text: "Continue" width: 156 height: 32 min_height: 32 margin: 0.0
                                padding: Inset{left: 16. right: 16. top: 0. bottom: 0.}
                                draw_bg +: {
                                    enabled_f: instance(0.0)
                                    light: instance(0.0)
                                    pixel: fn() {
                                        let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                                        let off = mix(#x38383A, #xE5E5EA, self.light)
                                        let live = mix(off, #x0060CC, self.enabled_f)
                                        sdf.box(0.5, 0.5, self.rect_size.x - 1.0, self.rect_size.y - 1.0, 4.0)
                                        sdf.fill(mix(live, #x0060CC, self.hover * self.enabled_f))
                                        return sdf.result
                                    }
                                }
                                draw_text +: {
                                    enabled_f: instance(0.0)
                                    light: instance(0.0)
                                    text_style: mod.widgets.ux.TEXT_BODY{}
                                    get_color: fn() {
                                        let off = mix(#xAEAEB2, #x636366, self.light)
                                        return mix(off, #xFFFFFF, self.enabled_f)
                                    }
                                }
                            }
                        }
                    }}
            }
        }
    }
}

#[derive(Script, ScriptHook, Widget)]
pub struct HardwareScreen {
    #[deref]
    view: View,
    /// Model currently mounted in the mirror, so it is opened once.
    #[rust]
    loaded_urdf: Option<(std::path::PathBuf, std::path::PathBuf)>,
    #[rust]
    inspector_available: Option<bool>,
    #[rust]
    fit_pending: bool,
    #[rust]
    viewport_aspect: f64,
    #[rust]
    last_view_distance: f32,
}

impl Widget for HardwareScreen {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        let actions = cx.capture_actions(|cx| self.view.handle_event(cx, event, scope));
        if self.view.button(cx, ids!(hardware_reset)).clicked(&actions) {
            self.view.flex_panels(cx, ids!(content)).reset(cx);
        }
        if self.view.button(cx, ids!(hardware_fit)).clicked(&actions) {
            fit_robot(cx, &self.view);
        }
        cx.extend_actions(actions);
    }
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let step = self.view.draw_walk(cx, scope, walk);
        if step.is_ok() {
            let widget = self.view.widget(cx, ids!(content.robot_panel.content.mirror.viewport));
            let rect = widget.area().rect(cx);
            if rect.size.x > 1.0 && rect.size.y > 1.0 {
                let aspect = (rect.size.x / rect.size.y).clamp(0.4, 1.0);
                if self.fit_pending {
                    fit_robot(cx, &self.view);
                    self.fit_pending = false;
                } else if self.viewport_aspect > 0.0 && (self.viewport_aspect - aspect).abs() > 0.001 {
                    if let Some(mut viewer) = widget.borrow_mut::<makepad_urdf_player::robot_view::RobotView>() {
                        let distance = viewer.view_distance() * (self.viewport_aspect / aspect) as f32;
                        viewer.set_view_distance(cx, distance);
                    }
                }
                self.viewport_aspect = aspect;
                if let Some(viewer) = widget.borrow::<makepad_urdf_player::robot_view::RobotView>() {
                    self.last_view_distance = viewer.view_distance();
                }
            }
        }
        step
    }
}

fn fit_robot(cx: &mut Cx, root: &View) {
    let widget = root.widget(cx, ids!(content.robot_panel.content.mirror.viewport));
    let rect = widget.area().rect(cx);
    if let Some(mut viewer) = widget.borrow_mut::<makepad_urdf_player::robot_view::RobotView>() {
        if let Some(robot) = viewer.robot() {
            let (min, max) = robot.bounds_world();
            let center = (min + max) * 0.5;
            let radius = (max - min).length() * 0.5;
            let aspect = if rect.size.y > 0.0 {
                (rect.size.x / rect.size.y).clamp(0.4, 1.0) as f32
            } else {
                1.0
            };
            viewer.set_camera_target(cx, [center.x, center.y, center.z]);
            viewer.set_view_angles(cx, 0.72, -0.22);
            viewer.set_view_distance(cx, radius * 3.1 / aspect);
        }
    };
}

const STEP_IDS: [&[LiveId]; 5] = [
    ids!(stepper.step_0),
    ids!(stepper.step_1),
    ids!(stepper.step_2),
    ids!(stepper.step_3),
    ids!(stepper.step_4),
];
const LINE_IDS: [&[LiveId]; 4] = [
    ids!(stepper.line_0),
    ids!(stepper.line_1),
    ids!(stepper.line_2),
    ids!(stepper.line_3),
];
const JOINT_IDS: [&[LiveId]; 6] = [
    ids!(content.camera_a_panel.content.calib.rows.joint_0),
    ids!(content.camera_a_panel.content.calib.rows.joint_1),
    ids!(content.camera_a_panel.content.calib.rows.joint_2),
    ids!(content.camera_a_panel.content.calib.rows.joint_3),
    ids!(content.camera_a_panel.content.calib.rows.joint_4),
    ids!(content.camera_a_panel.content.calib.rows.joint_5),
];

impl HardwareScreenRef {
    pub fn restore_view_distance(&self, cx: &mut Cx) {
        if let Some(inner) = self.borrow() {
            let widget = inner.view.widget(cx, ids!(content.robot_panel.content.mirror.viewport));
            if inner.last_view_distance > 0.0 {
                if let Some(mut viewer) = widget.borrow_mut::<makepad_urdf_player::robot_view::RobotView>() {
                    viewer.set_view_distance(cx, inner.last_view_distance);
                }
            }
        }
    }
    /// Hide the simulation's unused inspector without changing the user's layout.
    pub fn set_inspector_available(&self, cx: &mut Cx, available: bool) {
        if let Some(mut inner) = self.borrow_mut() {
            if inner.inspector_available != Some(available) {
                inner.inspector_available = Some(available);
                inner.fit_pending = true;
                inner.view.flex_panels(cx, ids!(content)).set_panel_available(cx, 1, available);
            }
        }
    }
    pub fn sync(&self, cx: &mut Cx, state: &HardwareState) {
        let light = crate::ui::frame::light_mode();
        let Some(mut inner) = self.borrow_mut() else {
            return;
        };
        let want = state.robot_urdf.clone();
        let need_load = inner.loaded_urdf != want;
        if need_load {
            inner.loaded_urdf = want.clone();
            inner.fit_pending = true;
        }
        let root = &mut inner.view;

        let nano = state.nano.as_ref();
        root.widget(cx, ids!(stepper)).set_visible(cx, nano.is_none());
        root.widget(cx, ids!(content.camera_a_panel.content.calib.nano_controls)).set_visible(cx, nano.is_some());
        root.widget(cx, ids!(content.camera_a_panel.content.calib.actions)).set_visible(cx, nano.is_none());
        root.widget(cx, ids!(content.camera_a_panel.content.calib.banner)).set_visible(cx, nano.is_none());
        root.button(cx, ids!(supported_robots.nano_select)).set_text(cx, if nano.is_some() { "Adora Nano · Feetech base ✓" } else { "Adora Nano · Feetech base" });
        if let Some(nano) = nano {
            root.widget(cx, ids!(content.camera_a_panel.content.calib.nano_controls.serial_row)).set_visible(cx, !nano.active);
            let title = if nano.active { if nano.mock { "Bus test connected" } else { "Base connected" } } else if nano.phase == "error" { "Connection failed" } else { "Connect mobile base" };
            root.label(cx, ids!(content.camera_a_panel.content.calib.nano_controls.driver_state)).set_text(cx,title);
            root.label(cx, ids!(content.camera_a_panel.content.calib.nano_controls.wheel_commands))
                .set_text(cx, &format!("Ticks 13: {:+} · 14: {:+} · 15: {:+}\n{} writes · no position feedback", nano.command_ticks[0], nano.command_ticks[1], nano.command_ticks[2], nano.acks));
            let help = if nano.phase == "error" { nano.detail.clone() }
                else if nano.active && nano.mock { "Command test only · no simulated motion.\nL1 hold to send · release to stop".into() }
                else if nano.active { "L1 hold to drive · release to stop\nLeft stick: drive / strafe · Right stick: turn".into() }
                else { "USB serial · Feetech mobile base".into() };
            root.label(cx, ids!(content.camera_a_panel.content.calib.nano_controls.help)).set_text(cx, &help);
            root.widget(cx, ids!(content.camera_a_panel.content.calib.nano_controls.connection_row.connect)).set_visible(cx, !nano.active);
            root.widget(cx, ids!(content.camera_a_panel.content.calib.nano_controls.connection_row.test)).set_visible(cx, !nano.active);
            root.widget(cx, ids!(content.camera_a_panel.content.calib.nano_controls.connection_row.stop)).set_visible(cx, nano.active);
            root.button(cx, ids!(content.camera_a_panel.content.calib.nano_controls.connection_row.connect)).set_enabled(cx, !nano.active);
            root.button(cx, ids!(content.camera_a_panel.content.calib.nano_controls.connection_row.test)).set_enabled(cx, !nano.active);
            root.button(cx, ids!(content.camera_a_panel.content.calib.nano_controls.connection_row.stop)).set_enabled(cx, nano.active);
        }

        root.label(cx, ids!(content.robot_panel.head.title))
            .set_text(
                cx,
                if state.live_angles.is_empty() {
                    "Reference model"
                } else {
                    "Live mirror"
                },
            );
        root.label(cx, ids!(content.camera_a_panel.head.title))
            .set_text(
                cx,
                if state.joints.is_empty() && !state.can_advance {
                    "Connection status"
                } else {
                    "Joint calibration"
                },
            );
        // The mirror renders the robot itself, so the model is opened once and
        // then only posed. With no arm attached there are no live angles yet,
        // and it simply sits at its rest pose.
        let viewer = root.widget(cx, ids!(content.robot_panel.content.mirror.viewport));
        if need_load {
            if let (Some((urdf, assets)), Some(mut rv)) = (
                want.as_ref(),
                viewer.borrow_mut::<makepad_urdf_player::robot_view::RobotView>(),
            ) {
                if let Err(e) = rv.load_robot(&urdf.to_string_lossy(), &assets.to_string_lossy()) {
                    ::log::error!("hardware: urdf {} failed to load: {e}", urdf.display());
                }
            }
        }
        if !state.live_angles.is_empty() {
            if let Some(mut rv) = viewer.borrow_mut::<makepad_urdf_player::robot_view::RobotView>()
            {
                rv.set_joint_angles(cx, &state.live_angles);
            }
        }
        script_apply_eval!(cx, root, { draw_bg +: { light: #(light) } });
        let sky = if light > 0.5 {
            dorobot_ux::tokens::pal::VOID_L
        } else {
            dorobot_ux::tokens::pal::DEEP_D
        };
        let ground = if light > 0.5 {
            dorobot_ux::tokens::pal::VOID_L
        } else {
            dorobot_ux::tokens::pal::LIFT_D
        };
        let grid = if light > 0.5 {
            vec4(0.77, 0.78, 0.81, 1.0)
        } else {
            vec4(0.27, 0.28, 0.31, 1.0)
        };
        let mut viewport = root.widget(cx, ids!(content.robot_panel.content.mirror.viewport));
        script_apply_eval!(cx, viewport, { sky_horizon: #(sky) sky_zenith: #(sky) ground_color: #(ground) ground2_color: #(ground) grid_color: #(grid) });
        apply_light_in(
            cx,
            root,
            &[
                (ids!(hardware_reset), Themed::Both),
                (ids!(hardware_fit), Themed::Both),
                (
                    ids!(content.camera_a_panel.content.calib.banner),
                    Themed::Bg,
                ),
                (
                    ids!(content.camera_a_panel.content.calib.banner.ring),
                    Themed::Bg,
                ),
                (
                    ids!(content.camera_a_panel.content.calib.banner.ring.ring_text),
                    Themed::Text,
                ),
                (
                    ids!(
                        content
                            .camera_a_panel
                            .content
                            .calib
                            .banner
                            .btext
                            .instruction
                    ),
                    Themed::Text,
                ),
                (
                    ids!(content.camera_a_panel.content.calib.banner.btext.subline),
                    Themed::Text,
                ),
                (
                    ids!(content.camera_a_panel.content.calib.actions.btn_restart),
                    Themed::Both,
                ),
                (
                    ids!(content.camera_a_panel.content.calib.actions.btn_continue),
                    Themed::Both,
                ),
            ],
            light,
        );
        for p in LINE_IDS {
            let mut w = root.widget(cx, p);
            if !w.is_empty() {
                script_apply_eval!(cx, w, { draw_bg +: { light: #(light) } });
            }
        }

        // ---- stepper -------------------------------------------------------
        for (i, path) in STEP_IDS.iter().enumerate() {
            let item = root.widget(cx, path);
            if item.is_empty() {
                continue;
            }
            match state.steps.get(i) {
                Some(step) => {
                    item.set_visible(cx, true);
                    apply_light(
                        cx,
                        &item,
                        &[
                            (ids!(dot), Themed::Bg),
                            (ids!(dot.num), Themed::Text),
                            (ids!(caption), Themed::Text),
                        ],
                        light,
                    );
                    let s = match step.state {
                        StepState::Pending => 0.0,
                        StepState::Active => 1.0,
                        StepState::Done => 2.0,
                    };
                    item.label(cx, ids!(caption)).set_text(cx, &step.title);
                    item.label(cx, ids!(dot.num))
                        .set_text(cx, &format!("{}", i + 1));
                    // The number is replaced by the checkmark once complete.
                    item.widget(cx, ids!(dot.num))
                        .set_visible(cx, step.state != StepState::Done);

                    let mut dot = item.widget(cx, ids!(dot));
                    script_apply_eval!(cx, dot, { draw_bg +: { state: #(s) } });
                    let mut num = item.widget(cx, ids!(dot.num));
                    script_apply_eval!(cx, num, { draw_text +: { state: #(s) } });
                    let mut cap = item.widget(cx, ids!(caption));
                    script_apply_eval!(cx, cap, { draw_text +: { state: #(s) } });
                }
                None => item.set_visible(cx, false),
            }
        }
        for (i, path) in LINE_IDS.iter().enumerate() {
            let mut connector = root.widget(cx, path);
            if connector.is_empty() {
                continue;
            }
            // A connector lights up once the step to its left is complete.
            let done = state
                .steps
                .get(i)
                .map(|s| s.state == StepState::Done)
                .unwrap_or(false);
            // NB: `v` as an interpolation name collides with script_apply_eval!'s
            // internal Vec<ScriptValue> binding — keep these names distinctive.
            let lit = if done { 1.0 } else { 0.0 };
            script_apply_eval!(cx, connector, { draw_bg +: { done: #(lit) } });
        }

        // ---- progress banner ----------------------------------------------
        let done_n = state.joints_done();
        let total = state.joints.len();
        // Port discovery has no calibration progress yet. A 0/0 ring looks
        // like an incomplete task even though no joint readings are available.
        root.widget(cx, ids!(content.camera_a_panel.content.calib.banner.ring))
            .set_visible(cx, total > 0);
        root.widget(
            cx,
            ids!(content.camera_a_panel.content.calib.banner.btext.subline),
        )
        .set_visible(cx, total > 0);
        root.button(
            cx,
            ids!(content.camera_a_panel.content.calib.actions.btn_restart),
        )
        .set_text(
            cx,
            if total == 0 && !state.can_advance {
                "Check connections"
            } else {
                "Restart step"
            },
        );
        let frac = if total == 0 {
            0.0
        } else {
            done_n as f64 / total as f64
        };
        root.label(
            cx,
            ids!(
                content
                    .camera_a_panel
                    .content
                    .calib
                    .banner
                    .btext
                    .instruction
            ),
        )
        .set_text(cx, &state.instruction);
        root.label(
            cx,
            ids!(content.camera_a_panel.content.calib.banner.btext.subline),
        )
        .set_text(cx, &format!("{}/{} joints complete", done_n, total));
        root.label(
            cx,
            ids!(content.camera_a_panel.content.calib.banner.ring.ring_text),
        )
        .set_text(cx, &format!("{}/{}", done_n, total));
        let mut ring = root.widget(cx, ids!(content.camera_a_panel.content.calib.banner.ring));
        script_apply_eval!(cx, ring, { draw_bg +: { frac: #(frac) } });

        // ---- joint rows ----------------------------------------------------
        for (i, path) in JOINT_IDS.iter().enumerate() {
            let row = root.widget(cx, path);
            if row.is_empty() {
                continue;
            }
            match state.joints.get(i) {
                Some(j) => {
                    row.set_visible(cx, true);
                    {
                        let mut r = row.clone();
                        script_apply_eval!(cx, r, { draw_bg +: { light: #(light) } });
                    }
                    apply_light(
                        cx,
                        &row,
                        &[
                            (ids!(pip), Themed::Bg),
                            (ids!(idx), Themed::Text),
                            (ids!(jname), Themed::Text),
                            (ids!(bar), Themed::Bg),
                            (ids!(readout), Themed::Text),
                        ],
                        light,
                    );
                    let done = j.progress == JointProgress::Done;
                    row.label(cx, ids!(idx)).set_text(cx, &format!("{}", i + 1));
                    row.label(cx, ids!(jname)).set_text(cx, &j.name);

                    // Completed joints show the learned range; the rest nag.
                    let readout = if done {
                        format!("{:.1}°   {:.1}°", j.min_deg, j.max_deg)
                    } else {
                        j.progress.hint().to_string()
                    };
                    row.label(cx, ids!(readout)).set_text(cx, &readout);

                    let d = if done { 1.0 } else { 0.0 };
                    let frac = j.swept.clamp(0.0, 1.0) as f64;
                    let mut pip = row.widget(cx, ids!(pip));
                    script_apply_eval!(cx, pip, { draw_bg +: { done: #(d) } });
                    let mut bar = row.widget(cx, ids!(bar));
                    script_apply_eval!(cx, bar, { draw_bg +: { frac: #(frac) done: #(d) } });
                    let mut ro = row.widget(cx, ids!(readout));
                    script_apply_eval!(cx, ro, { draw_text +: { warn: #(1.0 - d) } });
                }
                None => row.set_visible(cx, false),
            }
        }

        // ---- continue gate --------------------------------------------------
        let enabled = if state.can_continue() { 1.0 } else { 0.0 };
        let btn = root.button(
            cx,
            ids!(content.camera_a_panel.content.calib.actions.btn_continue),
        );
        btn.set_enabled(cx, state.can_continue());
        let mut btn_w = root.widget(
            cx,
            ids!(content.camera_a_panel.content.calib.actions.btn_continue),
        );
        script_apply_eval!(cx, btn_w, {
            draw_bg +: { enabled_f: #(enabled) }
            draw_text +: { enabled_f: #(enabled) }
        });
    }
}

impl HardwareScreenRef {
    /// Wizard buttons, as intents the caller can dispatch. They were drawn but
    /// never read, so pressing them did nothing.
    pub fn wizard_intent(&self, cx: &mut Cx, actions: &Actions) -> Option<Intent> {
        let mut inner = self.borrow_mut()?;
        let v = &mut inner.view;
        if v.button(cx, ids!(supported_robots.simulation_select)).clicked(actions) {
            return Some(Intent::SelectSimulation(true));
        }
        if v.button(cx, ids!(supported_robots.nano_select)).clicked(actions) {
            return Some(Intent::SelectNanoBase(true));
        }
        if v.button(cx, ids!(supported_robots.rig_select)).clicked(actions) {
            return Some(Intent::SelectNanoBase(false));
        }
        if v.button(cx, ids!(content.camera_a_panel.content.calib.nano_controls.serial_row.scan)).clicked(actions) {
            let (ports, _) = crate::api::files::serial_ports();
            let input = v.text_input(cx, ids!(content.camera_a_panel.content.calib.nano_controls.serial_row.port));
            // Repeated clicks cycle candidates; discovery alone never connects.
            let current = input.text();
            if !ports.is_empty() {
                let next = ports.iter().position(|p| p == &current).map_or(0, |i| (i+1)%ports.len());
                input.set_text(cx, &ports[next]);
            } else { input.set_text(cx, ""); }
            return Some(Intent::SelectNanoBase(true));
        }
        if v.button(cx, ids!(content.camera_a_panel.content.calib.nano_controls.connection_row.connect)).clicked(actions) {
            return Some(Intent::ConnectNanoBase { mock: false, port: v.text_input(cx, ids!(content.camera_a_panel.content.calib.nano_controls.serial_row.port)).text() });
        }
        if v.button(cx, ids!(content.camera_a_panel.content.calib.nano_controls.connection_row.test)).clicked(actions) {
            return Some(Intent::ConnectNanoBase { mock: true, port: String::new() });
        }
        if v.button(cx, ids!(content.camera_a_panel.content.calib.nano_controls.connection_row.stop)).clicked(actions) {
            return Some(Intent::DisconnectNanoBase);
        }
        if v.button(
            cx,
            ids!(content.camera_a_panel.content.calib.actions.btn_restart),
        )
        .clicked(actions)
        {
            return Some(Intent::WizardRestartStep);
        }
        if v.button(
            cx,
            ids!(content.camera_a_panel.content.calib.actions.btn_continue),
        )
        .clicked(actions)
        {
            return Some(Intent::WizardAdvance);
        }
        None
    }
}
