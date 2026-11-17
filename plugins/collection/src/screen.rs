//! Recording console — live capture with review-before-save.
//!
//! Mirrors `docs/ux/ux-03-record.png`. Camera panes and joint traces are
//! procedural placeholders here: real pixels arrive when the live dora feed is
//! wired, and the trace shader is seeded per joint so renders stay comparable.

use dorobot_ux::kit::SparkWidgetRefExt;
use makepad_widgets::*;

use dorobot_studio_core::api::{RecordState, TakeVerdict};
use dorobot_studio_core::ui::frame::{apply_light, apply_light_in, theme_chip, theme_panel_head, Themed};

script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*
    use mod.widgets.ux.*

    // Live camera pane: dark stage + LIVE badge. Swapped for a texture-backed
    // Image once the capture pipeline exists.
    let CameraPane = mod.widgets.ux.Card{
        width: Fill height: Fill
        flow: Down
        head := mod.widgets.ux.PanelHead{ title +: { text: "cam" } }
        stage := View{
            width: Fill height: Fill
            flow: Overlay
            align: Align{x: 0.5 y: 0.5}
            show_bg: true
            draw_bg +: {
                light: instance(0.0)
                pixel: fn() {
                    let p = self.pos
                    // soft vignette so the pane reads as a camera stage
                    let d = length(p - vec2(0.5, 0.45))
                    let core = mix(#x1C1C1E, #x2C2C2E, p.y)
                    let lit = mix(core, #x000000, clamp(d * 0.9, 0.0, 1.0))
                    return mix(lit, mix(#x38383A, #x1C1C1E, clamp(d, 0.0, 1.0)), self.light * 0.0)
                }
            }
            preview := Image{ width: Fill height: Fill }
            waiting := Label{width: Fill align: Align{x: 0.5} margin: Inset{left: 8. right: 8.} text: "No camera frames"
                draw_text +: {color: #xAEAEB2 text_style: mod.widgets.ux.TEXT_META{}}}
            badge := RoundedView{
                width: Fit height: Fit
                padding: Inset{left: 8. right: 9. top: 3. bottom: 3.}
                margin: Inset{left: 10. bottom: 10.}
                align: Align{y: 0.5}
                flow: Right
                spacing: 6.0
                show_bg: true
                draw_bg +: {
                    color: #x14181FCC
                    border_radius: 0.0
                }
                dot := View{
                    width: 7 height: 7
                    show_bg: true
                    draw_bg +: {
                        pixel: fn() {
                            let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                            sdf.circle(3.5, 3.5, 3.0)
                            sdf.fill(#xD70015)
                            return sdf.result
                        }
                    }
                }
                Label{
                    text: "LIVE"
                    draw_text +: { color: #xE5E5EA text_style: mod.widgets.ux.TEXT_CHIP{} }
                }
            }
        }
    }

    // One joint trace row: name, value, waveform, limits.
    let TraceRow = View{
        width: Fill height: 26
        flow: Right
        align: Align{y: 0.5}
        spacing: 10.0
        padding: Inset{left: 14. right: 14.}
        jn := Label{
            width: 30 text: "J1"
            draw_text +: {
                light: instance(0.0)
                text_style: mod.widgets.ux.TEXT_META{}
                get_color: fn() { return mix(#xD1D1D6, #x636366, self.light) }
            }
        }
        jv := Label{
            width: 74 text: "+0.000"
            draw_text +: {
                light: instance(0.0)
                text_style: mod.widgets.ux.TEXT_META{}
                get_color: fn() { return mix(#xD1D1D6, #x38383A, self.light) }
            }
        }
        wave := Spark{ width: Fill height: 18 }
        jl := Label{
            width: 112 text: ""
            draw_text +: {
                light: instance(0.0)
                text_style: mod.widgets.ux.TEXT_META{}
                get_color: fn() { return mix(#xAEAEB2, #x636366, self.light) }
            }
        }
    }

    let FilmCell = View{
        preview := Image{ width: Fill height: Fill }
        width: 118 height: 74
        margin: Inset{right: 10.}
        show_bg: true
        draw_bg +: {
            light: instance(0.0)
            pixel: fn() {
                let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                sdf.box(0.0, 0.0, self.rect_size.x, self.rect_size.y, 0.5)
                sdf.fill(mix(mix(#x38383A, #x2C2C2E, self.pos.y),
                             mix(#xF2F2F7, #xE5E5EA, self.pos.y), self.light))
                // sprocket strips top and bottom
                sdf.box(0.0, 0.0, self.rect_size.x, 7.0, 0.0)
                sdf.fill(mix(#x1C1C1E, #xD1D1D6, self.light))
                sdf.box(0.0, self.rect_size.y - 7.0, self.rect_size.x, 7.0, 0.0)
                sdf.fill(mix(#x1C1C1E, #xD1D1D6, self.light))
                return sdf.result
            }
        }
    }

    mod.widgets.RecordScreenBase = #(RecordScreen::register_widget(vm))
    mod.widgets.RecordScreen = set_type_default() do mod.widgets.RecordScreenBase{
        width: Fill height: Fill
        flow: Down
        spacing: 12.0
        padding: Inset{left: 14. right: 14. top: 12. bottom: 12.}
        show_bg: true
        draw_bg +: {
            light: instance(0.0)
            pixel: fn() { return mix(#x111114, #xF2F2F7, self.light) }
        }

        // ---- session bar ----
        sbar := View{
            width: Fill height: 40
            flow: Right
            align: Align{y: 0.5}
            spacing: 16.0
            padding: Inset{left: 4. right: 6.}
            profile := mod.widgets.ux.Chip{
                draw_bg +: {tone: 0.0}
                label +: { text: "profile" }
            }
            task := Label{
                width: Fill max_lines: 1 text_overflow: Ellipsis
                text: ""
                draw_text +: {
                    light: instance(0.0)
                    text_style: mod.widgets.ux.TEXT_BODY{}
                    get_color: fn() { return mix(#xD1D1D6, #x38383A, self.light) }
                }
            }
            counter := Label{
                text: "EP 0 / 0"
                draw_text +: {
                    light: instance(0.0)
                    text_style: mod.widgets.ux.FONT_MEDIUM{font_size: 17.0}
                    get_color: fn() { return mix(#xE5E5EA, #x2C2C2E, self.light) }
                }
            }
            elapsed := Label{
                text: "00:00.0"
                margin: Inset{left: 14.}
                draw_text +: {
                    light: instance(0.0)
                    text_style: mod.widgets.ux.FONT_MEDIUM{font_size: 17.0}
                    get_color: fn() { return mix(#xD1D1D6, #x636366, self.light) }
                }
            }
        }

        // ---- main split ----
        body := View{
            width: Fill height: Fill
            flow: Right
            spacing: 12.0

            left := View{
                width: Fill height: Fill
                flow: Down
                spacing: 12.0
                cams := View{
                    width: Fill height: Fill
                    flow: Right
                    spacing: 12.0
                    camera_empty := mod.widgets.ux.Card{width: Fill height: Fill flow: Down align: Align{x: 0.5 y: 0.5} padding: 24.0 spacing: 10.0
                        title := Label{width: Fill align: Align{x: 0.5} text: "Camera previews" draw_text +: {color: mod.theme.color_text text_style: mod.widgets.ux.TEXT_TITLE{}}}
                        detail := Label{width: Fill align: Align{x: 0.5} text: "Connect cameras in Hardware to preview your recording." draw_text +: {light: instance(1.0) text_style: mod.widgets.ux.TEXT_META{} get_color: fn() {return mix(#xAEAEB2,#x636366,self.light)}}}
                    }
                    cam_0 := CameraPane{}
                    cam_1 := CameraPane{}
                }
                traces := mod.widgets.ux.Card{
                    width: Fill height: 228
                    flow: Down
                    tr_head := mod.widgets.ux.PanelHead{ title +: { text: "Joint state" } }
                    tr_body := View{
                        width: Fill height: Fill flow: Down
                        padding: Inset{top: 6. bottom: 6.}
                        empty := Label{width: Fill margin: Inset{left: 14. top: 16. right: 14.} text: "Joint readings appear when a robot is connected."
                            draw_text +: {light: instance(1.0) text_style: mod.widgets.ux.TEXT_META{} get_color: fn() {return mix(#xAEAEB2,#x636366,self.light)}}}
                        trace_0 := TraceRow{ wave +: { draw_bg +: {seed: 0.0} } }
                        trace_1 := TraceRow{ wave +: { draw_bg +: {seed: 1.0} } }
                        trace_2 := TraceRow{ wave +: { draw_bg +: {seed: 2.0} } }
                        trace_3 := TraceRow{ wave +: { draw_bg +: {seed: 3.0} } }
                        trace_4 := TraceRow{ wave +: { draw_bg +: {seed: 4.0} } }
                        trace_5 := TraceRow{ wave +: { draw_bg +: {seed: 5.0} } }
                    }
                }
            }

            right := View{
                width: 330 height: Fill
                flow: Down
                spacing: 12.0
                view3d := mod.widgets.ux.Card{
                    width: Fill height: Fill
                    flow: Down
                    v3_head := mod.widgets.ux.PanelHead{ title +: { text: "3D View" } }
                    v3_body := View{
                        width: Fill height: Fill
                        align: Align{x: 0.5 y: 0.5}
                        show_bg: true
                        draw_bg +: {
                            light: instance(0.0)
                            pixel: fn() {
                                let p = self.pos * self.rect_size
                                let horizon = self.rect_size.y * 0.42
                                let below = step(horizon, p.y)
                                let ny = (p.y - horizon) / max(self.rect_size.y - horizon, 1.0)
                                let rows = fract(pow(max(ny, 0.0), 0.55) * 7.0)
                                let hline = (1.0 - step(0.05, rows)) * below
                                let dx = (p.x - self.rect_size.x * 0.5) / max(p.y - horizon, 1.0)
                                let vline = (1.0 - step(0.02, fract(dx * 2.5 + 0.5))) * below
                                let g = clamp(hline + vline, 0.0, 1.0) * 0.45
                                let base = mix(#x1C1C1E, #xF2F2F7, self.light)
                                return mix(base, mix(#x38383A, #xD1D1D6, self.light), g)
                            }
                        }
                        robot_view := mod.widgets.RobotView{keyboard_controls: false width: Fill height: Fill
                            sky_horizon: mod.theme.color_bg_app sky_zenith: mod.theme.color_bg_app
                            ground_color: mod.theme.color_bg_container ground2_color: mod.theme.color_bg_container
                            grid_color: mod.theme.color_text_disabled}
                        no_robot := Label{width: Fill align: Align{x: 0.5} margin: 14.0 text: "Choose a rig profile to preview your robot" draw_text.color: mod.theme.color_text}

                    }
                }
                session := mod.widgets.ux.Card{
                    width: Fill height: Fit
                    flow: Down
                    ss_head := mod.widgets.ux.PanelHead{ title +: { text: "Session" } }
                    ss_body := View{
                        width: Fill height: Fit flow: Down
                        padding: Inset{left: 14. right: 14. top: 8. bottom: 10.}
                        spacing: 10.0
                        prog_row := View{
                            width: Fill height: Fit flow: Right align: Align{y: 0.5}
                            prog_label := Label{
                                width: Fill text: "PROGRESS"
                                draw_text +: {
                                    light: instance(0.0)
                                    text_style: mod.widgets.ux.TEXT_CHIP{}
                                    get_color: fn() { return mix(#xAEAEB2, #x636366, self.light) }
                                }
                            }
                            prog_val := Label{
                                text: "0 / 0"
                                draw_text +: {
                                    light: instance(0.0)
                                    text_style: mod.widgets.ux.TEXT_META{}
                                    get_color: fn() { return mix(#xD1D1D6, #x38383A, self.light) }
                                }
                            }
                        }
                        prog_bar := View{
                            width: Fill height: 8
                            show_bg: true
                            draw_bg +: {
                                frac: instance(0.0)
                                light: instance(0.0)
                                pixel: fn() {
                                    let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                                    sdf.box(0.0, 0.0, self.rect_size.x, 8.0, 4.0)
                                    sdf.fill(mix(#x38383A, #xE5E5EA, self.light))
                                    sdf.box(0.0, 0.0, self.rect_size.x * self.frac, 8.0, 4.0)
                                    sdf.fill(#x007AFF)
                                    return sdf.result
                                }
                            }
                        }
                        tally := Label{
                            text: "Saved 0 · Discarded 0"
                            draw_text +: {
                                light: instance(0.0)
                                text_style: mod.widgets.ux.TEXT_META{}
                                get_color: fn() { return mix(#xD1D1D6, #x636366, self.light) }
                            }
                        }
                        cues := Label{
                            text: "SOUND CUES  ON"
                            draw_text +: {
                                light: instance(0.0)
                                text_style: mod.widgets.ux.TEXT_CHIP{}
                                get_color: fn() { return mix(#xAEAEB2, #x636366, self.light) }
                            }
                        }
                    }
                }
            }
        }

        // ---- review strip ----
        review := mod.widgets.ux.Card{
            width: Fill height: 132
            flow: Down
            rv_head := mod.widgets.ux.PanelHead{ title +: { text: "Last episode replay" } }
            rv_body := View{
                width: Fill height: Fill
                flow: Right
                align: Align{y: 0.5}
                padding: Inset{left: 14. right: 14.}
                no_thumbnails := Label{width: Fill text: "No camera preview for this take"
                    draw_text +: {light: instance(1.0) text_style: mod.widgets.ux.TEXT_META{} get_color: fn() {return mix(#xAEAEB2,#x636366,self.light)}}}
                film_0 := FilmCell{}
                film_1 := FilmCell{}
                film_2 := FilmCell{}
                film_3 := FilmCell{}
                Filler{}
                verdict := mod.widgets.ux.Chip{
                    draw_bg +: {tone: 1.0}
                    label +: { text: "ready to save" draw_text +: {tone: 1.0} }
                }
            }
        }

        // ---- transport ----
        transport := View{
            width: Fill height: 78
            flow: Right
            align: Align{y: 0.5}
            spacing: 14.0
            padding: Inset{left: 8. right: 8.}

            btn_discard := mod.widgets.nx.NxBtn{height: 48 text: "Discard last"}
            btn_rerecord := mod.widgets.nx.NxBtn{height: 48 text: "Re-record"}
            Filler{}
            btn_record := mod.widgets.nx.Primary{
                text: "Record" width: 132 height: 48
                draw_bg +: {recording: instance(0.0) pixel: fn() {
                    let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                    sdf.box(0.5, 0.5, self.rect_size.x-1.0, self.rect_size.y-1.0, mod.widgets.nx.BUTTON_RADIUS)
                    sdf.fill(mix(#xC40013, #xD1D1D6, self.disabled))
                    return sdf.result
                }}
                draw_text +: {get_color: fn() {return mix(#xFFFFFF, #x636366, self.disabled)}}
            }
            Filler{}
            btn_save := mod.widgets.nx.Primary{height: 48 text: "Save episode"
                draw_bg +: {pixel: fn() {
                    let sdf = Sdf2d.viewport(self.pos * self.rect_size)
                    sdf.box(0.5,0.5,self.rect_size.x-1.0,self.rect_size.y-1.0,mod.widgets.nx.BUTTON_RADIUS)
                    sdf.fill(mix(#x0060CC,#xD1D1D6,self.disabled)); return sdf.result
                }}
                draw_text +: {get_color: fn() {return mix(#xFFFFFF,#x636366,self.disabled)}}
            }
        }
    }
}

#[derive(Script, ScriptHook, Widget)]
pub struct RecordScreen {
    #[rust]
    loaded_urdf: Option<(std::path::PathBuf, std::path::PathBuf)>,
    #[rust]
    model_loader: dorobot_studio_core::data::robot_model::RobotModelLoader,
    #[rust]
    model_error: Option<String>,
    #[rust]
    latest_pose: Vec<f32>,
    #[deref]
    view: View,
}

impl Widget for RecordScreen {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        if matches!(event, Event::Signal) { self.finish_model(cx); }
        self.view.handle_event(cx, event, scope);
    }
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        self.view.draw_walk(cx, scope, walk)
    }
}

impl RecordScreen {
    fn finish_model(&mut self, cx: &mut Cx) {
        let Some(ready) = self.model_loader.take_ready() else { return; };
        match ready {
            Ok(robot) => {
                let widget = self.view.widget(cx, ids!(body.right.view3d.v3_body.robot_view));
                if let Some(mut viewer) = widget.borrow_mut::<makepad_urdf_player::robot_view::RobotView>() {
                    viewer.set_robot(cx, robot);
                    viewer.set_joint_angles(cx, &self.latest_pose);
                }
                self.model_error = None;
            }
            Err(error) => self.model_error = Some(format!("Could not load robot model: {error}")),
        }
        self.sync_model_status(cx);
        self.view.redraw(cx);
    }

    fn sync_model_status(&self, cx: &mut Cx) {
        let waiting = self.loaded_urdf.is_none() || self.model_loader.loading() || self.model_error.is_some();
        self.view.widget(cx, ids!(body.right.view3d.v3_body.robot_view)).set_visible(cx, !waiting);
        let label = self.view.widget(cx, ids!(body.right.view3d.v3_body.no_robot));
        label.set_visible(cx, waiting);
        label.set_text(cx, self.model_error.as_deref().unwrap_or(if self.model_loader.loading() {
            "Loading robot model…"
        } else { "Choose a rig profile to preview your robot" }));
    }
}

const TRACE_IDS: [&[LiveId]; 6] = [
    ids!(body.left.traces.tr_body.trace_0),
    ids!(body.left.traces.tr_body.trace_1),
    ids!(body.left.traces.tr_body.trace_2),
    ids!(body.left.traces.tr_body.trace_3),
    ids!(body.left.traces.tr_body.trace_4),
    ids!(body.left.traces.tr_body.trace_5),
];
const CAM_IDS: [&[LiveId]; 2] = [ids!(body.left.cams.cam_0), ids!(body.left.cams.cam_1)];
const FILM_IDS: [&[LiveId]; 4] = [
    ids!(review.rv_body.film_0),
    ids!(review.rv_body.film_1),
    ids!(review.rv_body.film_2),
    ids!(review.rv_body.film_3),
];

impl RecordScreenRef {
    pub fn sync(&self, cx: &mut Cx, st: &RecordState) {
        let light = dorobot_studio_core::ui::frame::light_mode();
        let Some(mut inner) = self.borrow_mut() else {
            return;
        };
        inner.latest_pose.clone_from(&st.pose);
        if inner.model_loader.request(st.robot_urdf.clone()) {
            inner.loaded_urdf = st.robot_urdf.clone();
            inner.model_error = None;
            let widget = inner.view.widget(cx, ids!(body.right.view3d.v3_body.robot_view));
            if let Some(mut viewer) = widget.borrow_mut::<makepad_urdf_player::robot_view::RobotView>() {
                viewer.clear_robot(cx);
            };
        } else { inner.finish_model(cx); }
        inner.sync_model_status(cx);
        let RecordScreen {
            view: root,
            ..
        } = &mut *inner;

        script_apply_eval!(cx, root, { draw_bg +: { light: #(light) } });
        apply_light_in(
            cx,
            root,
            &[
                (ids!(sbar.task), Themed::Text),
                (ids!(sbar.counter), Themed::Text),
                (ids!(sbar.elapsed), Themed::Text),
                (ids!(body.left.traces), Themed::Bg),
                (ids!(body.right.view3d), Themed::Bg),
                (ids!(body.right.view3d.v3_body), Themed::Bg),
                (ids!(body.right.session), Themed::Bg),
                (
                    ids!(body.right.session.ss_body.prog_row.prog_label),
                    Themed::Text,
                ),
                (
                    ids!(body.right.session.ss_body.prog_row.prog_val),
                    Themed::Text,
                ),
                (ids!(body.right.session.ss_body.prog_bar), Themed::Bg),
                (ids!(body.right.session.ss_body.tally), Themed::Text),
                (ids!(body.right.session.ss_body.cues), Themed::Text),
                (ids!(review), Themed::Bg),
                (ids!(transport.btn_discard), Themed::Both),
                (ids!(transport.btn_rerecord), Themed::Both),
            ],
            light,
        );
        for p in [
            ids!(body.left.traces.tr_head) as &[LiveId],
            ids!(body.right.view3d.v3_head),
            ids!(body.right.session.ss_head),
            ids!(review.rv_head),
        ] {
            let head = root.widget(cx, p);
            theme_panel_head(cx, &head, light);
        }
        if st.robot_urdf.is_some() {
            let viewer = root.widget(cx, ids!(body.right.view3d.v3_body.robot_view));
            if let Some(mut v) = viewer.borrow_mut::<makepad_urdf_player::robot_view::RobotView>() {
                if !st.pose.is_empty() {
                    v.set_joint_angles(cx, &st.pose);
                }
            };
        }
        for (i, p) in FILM_IDS.iter().enumerate() {
            let path = st.last_take.as_ref().and_then(|t| t.thumbnails.get(i));
            root.widget(cx, p).set_visible(cx, path.is_some());
            if let Some(path) = path {
                if let Ok(bytes) = std::fs::read(path) {
                    let _ = root
                        .widget(cx, p)
                        .image(cx, ids!(preview))
                        .load_png_from_data(cx, &bytes);
                }
            }

            let mut f = root.widget(cx, p);
            if !f.is_empty() {
                script_apply_eval!(cx, f, { draw_bg +: { light: #(light) } });
            }
        }

        // ---- session bar --------------------------------------------------
        root.label(cx, ids!(sbar.profile.label))
            .set_text(cx, &st.profile_label);
        root.label(cx, ids!(sbar.task))
            .set_text(cx, &format!("“{}”", st.task));
        root.label(cx, ids!(sbar.counter))
            .set_text(cx, &st.counter_label());
        root.label(cx, ids!(sbar.elapsed))
            .set_text(cx, &st.elapsed_label());
        let profile_chip = root.widget(cx, ids!(sbar.profile));
        theme_chip(cx, &profile_chip, light);

        root.widget(cx, ids!(body.left.cams.camera_empty))
            .set_visible(cx, st.cameras.is_empty());
        root.widget(cx, ids!(body.left.traces.tr_body.empty))
            .set_visible(cx, st.joints.is_empty());
        apply_light_in(
            cx,
            root,
            &[
                (ids!(body.left.cams.camera_empty), Themed::Bg),
                (ids!(body.left.cams.camera_empty.detail), Themed::Text),
                (ids!(body.left.traces.tr_body.empty), Themed::Text),
            ],
            light,
        );

        // ---- cameras ------------------------------------------------------
        for (i, p) in CAM_IDS.iter().enumerate() {
            let pane = root.widget(cx, p);
            if pane.is_empty() {
                continue;
            }
            match st.cameras.get(i) {
                Some(name) => {
                    pane.set_visible(cx, true);
                    pane.label(cx, ids!(head.title)).set_text(cx, name);
                    let has_preview = st.previews.contains_key(name);
                    pane.widget(cx, ids!(stage.waiting))
                        .set_visible(cx, !has_preview);
                    pane.widget(cx, ids!(stage.preview))
                        .set_visible(cx, has_preview);
                    pane.widget(cx, ids!(stage.badge))
                        .set_visible(cx, st.recording && has_preview);
                    if let Some(path) = st.previews.get(name) {
                        if let Ok(bytes) = std::fs::read(path) {
                            let _ = pane
                                .image(cx, ids!(stage.preview))
                                .load_png_from_data(cx, &bytes);
                        }
                    }
                    let mut c = pane.clone();
                    script_apply_eval!(cx, c, { draw_bg +: { light: #(light) } });
                    let head = pane.widget(cx, ids!(head));
                    theme_panel_head(cx, &head, light);
                }
                None => pane.set_visible(cx, false),
            }
        }

        // ---- joint traces --------------------------------------------------
        for (i, p) in TRACE_IDS.iter().enumerate() {
            let row = root.widget(cx, p);
            if row.is_empty() {
                continue;
            }
            match st.joints.get(i) {
                Some(j) => {
                    row.set_visible(cx, true);
                    row.label(cx, ids!(jn)).set_text(cx, &j.name);
                    row.label(cx, ids!(jv))
                        .set_text(cx, &format!("{:+.2}", j.value));
                    row.label(cx, ids!(jl))
                        .set_text(cx, &format!("{:.3}   {:.3}", j.min, j.max));
                    let values: Vec<f64> = j.history.iter().map(|v| *v as f64).collect();
                    row.spark(cx, ids!(wave))
                        .set(cx, &values, None, 0, light as f32);
                    apply_light(
                        cx,
                        &row,
                        &[
                            (ids!(jn), Themed::Text),
                            (ids!(jv), Themed::Text),
                            (ids!(jl), Themed::Text),
                            (ids!(wave), Themed::Bg),
                        ],
                        light,
                    );
                }
                None => row.set_visible(cx, false),
            }
        }

        // ---- session panel --------------------------------------------------
        root.label(cx, ids!(body.right.session.ss_body.prog_row.prog_val))
            .set_text(cx, &format!("{} / {}", st.saved, st.episode_target));
        root.label(cx, ids!(body.right.session.ss_body.tally))
            .set_text(
                cx,
                &format!("Saved {} · Discarded {}", st.saved, st.discarded),
            );
        root.label(cx, ids!(body.right.session.ss_body.cues))
            .set_text(
                cx,
                if st.sound_cues {
                    "SOUND CUES  ON"
                } else {
                    "SOUND CUES  OFF"
                },
            );
        let frac = st.progress();
        let mut bar = root.widget(cx, ids!(body.right.session.ss_body.prog_bar));
        script_apply_eval!(cx, bar, { draw_bg +: { frac: #(frac) light: #(light) } });

        if let Some(mut traces) = root.widget(cx, ids!(body.left.traces)).borrow_mut::<View>() {
            traces.walk.height = Size::Fixed(if st.joints.is_empty() { 128.0 } else { 228.0 });
        }
        let has_thumbnails = st
            .last_take
            .as_ref()
            .is_some_and(|take| !take.thumbnails.is_empty());
        if let Some(mut review) = root.widget(cx, ids!(review)).borrow_mut::<View>() {
            review.walk.height = Size::Fixed(if has_thumbnails { 132.0 } else { 112.0 });
        }
        root.widget(cx, ids!(review.rv_body.no_thumbnails))
            .set_visible(cx, !has_thumbnails);
        apply_light_in(
            cx,
            root,
            &[(ids!(review.rv_body.no_thumbnails), Themed::Text)],
            light,
        );

        // ---- review strip ---------------------------------------------------
        let has_take = st.last_take.is_some();
        root.widget(cx, ids!(review)).set_visible(cx, has_take);
        if let Some(take) = &st.last_take {
            let (txt, tone) = match take.verdict {
                TakeVerdict::ReadyToSave => ("ready to save", 1.0),
                TakeVerdict::Warning => ("check warnings", 4.0),
            };
            root.label(cx, ids!(review.rv_body.verdict.label))
                .set_text(cx, txt);
            let chip = root.widget(cx, ids!(review.rv_body.verdict));
            let mut c = chip.clone();
            script_apply_eval!(cx, c, { draw_bg +: { tone: #(tone) } });
            let mut cl = root.widget(cx, ids!(review.rv_body.verdict.label));
            script_apply_eval!(cx, cl, { draw_text +: { tone: #(tone) } });
            theme_chip(cx, &chip, light);
        }

        root.label(cx, ids!(sbar.profile.label)).set_text(
            cx,
            if st.profile_label.is_empty() {
                "No rig selected"
            } else {
                &st.profile_label
            },
        );
        root.label(cx, ids!(sbar.task)).set_text(
            cx,
            if st.task.is_empty() {
                "Choose a profile to set the recording task"
            } else {
                &st.task
            },
        );
        // ---- transport ------------------------------------------------------
        let rec = if st.recording { 1.0 } else { 0.0 };
        let mut btn = root.widget(cx, ids!(transport.btn_record));
        script_apply_eval!(cx, btn, { draw_bg +: { recording: #(rec) } });
        root.button(cx, ids!(transport.btn_record))
            .set_text(cx, if st.recording { "Stop" } else { "Record" });
        for (id, enabled) in [
            (
                ids!(transport.btn_record) as &[LiveId],
                st.recording || (st.ready_to_record && !st.profile_label.is_empty() && !st.busy && !has_take),
            ),
            (
                ids!(transport.btn_save),
                has_take && !st.busy && !st.recording,
            ),
            (
                ids!(transport.btn_discard),
                has_take && !st.busy && !st.recording,
            ),
            (
                ids!(transport.btn_rerecord),
                has_take && !st.busy && !st.recording,
            ),
        ] {
            root.button(cx, id).set_enabled(cx, enabled);
            root.widget(cx, id).set_disabled(cx, !enabled);
        }
    }
}
