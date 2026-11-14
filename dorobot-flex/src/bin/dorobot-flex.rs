// DoRobot Studio: real datasets and LeRobot capture, with explicit --demo fixtures.

use dorobot_flex::api::{files::FileBackend, mock::MockBackend, Backend, Intent, Screen};
use dorobot_flex::playback_controls::PlaybackAction;
use dorobot_flex::preferences;
use dorobot_flex::ui::{
    eval::EvalScreenWidgetRefExt, frame, hardware::HardwareScreenWidgetRefExt,
    library::LibraryScreenWidgetRefExt, play::PlayScreenWidgetRefExt,
    record::RecordScreenWidgetRefExt,
};
use dorobot_flex::widgets::time_series_plot::TimeSeriesPlotAction;
use dorobot_flex::widgets::timeline::TimelineAction;
use makepad_urdf_player::robot_view::RobotViewWidgetRefExt;
use makepad_widgets::*;

/// Transport tick. Matches the shipping player, and is independent of the
/// dataset's own fps: it advances a clock, and every view resolves itself
/// against that clock rather than counting frames of its own.
const PLAYBACK_TIMER_FPS: f64 = 60.0;

app_main!(App);

/// `--dataset <dir>` opens a dataset or library. Fixtures require `--demo`.
fn make_backend() -> Box<dyn Backend> {
    let args: Vec<String> = std::env::args().collect();
    let demo = args.iter().any(|a| a == "--demo")
        || std::env::current_exe()
            .ok()
            .is_some_and(|p| p.file_stem().is_some_and(|n| n == "ui_mock"));
    if demo {
        return Box::new(MockBackend::new());
    }
    let root = args
        .iter()
        .position(|a| a == "--dataset")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .or_else(|| std::env::var("DOROBOT_DATASETS").ok())
        .unwrap_or_else(|| format!("{}/../datasets", env!("CARGO_MANIFEST_DIR")));
    let prefer = args
        .iter()
        .position(|a| a == "--open")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str);
    let mut backend = FileBackend::with_open(root, prefer);
    let profile = args
        .iter()
        .position(|a| a == "--rig")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .or_else(|| std::env::var("DOROBOT_RIG").ok());
    if let Some(p) = profile {
        backend.dispatch(Intent::ConfigureRig(p.into()));
    }
    Box::new(backend)
}

script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*
    use mod.widgets.ux.*

    startup() do #(App::script_component(vm)){
        ui: Root{
            main_window := Window{
                window.title: "DoRobot Studio"
                window.inner_size: vec2(1536, 1024)
                pass.clear_color: mod.theme.color_bg_app

                body +: {
                    flow: Right width: Fill height: Fill
                    show_bg: true draw_bg.color: mod.theme.color_bg_app
                    sidebar := View{
                        width: 196 height: Fill flow: Down padding: 12.0 spacing: 12.0
                        show_bg: true draw_bg.color: mod.theme.color_bg_container
                        Label{ width: Fill height: 52 text: "DoRobot Studio"
                            draw_text +: {color: mod.theme.color_text text_style: mod.widgets.ux.TEXT_TITLE{}} }
                        nav := mod.widgets.ux.NavRail{ width: Fill height: Fill }
                        appearance := Button{width: Fill height: 44 text: "Appearance: Light"}
                    }
                    workspace := View{
                        width: Fill height: Fill flow: Down
                        page_header := View{
                            width: Fill height: Fit flow: Down padding: 18.0 spacing: 6.0
                            page_title := Label{width: Fill text: "Library"
                                draw_text +: {color: mod.theme.color_text text_style: mod.widgets.ux.TEXT_H1{}} }
                            status := Label{width: Fill height: Fit max_lines: 2 text_overflow: Ellipsis text: ""
                                draw_text +: {color: mod.theme.color_text text_style: mod.widgets.ux.TEXT_META{}} }
                        }
                        hub_form := View{ visible: false width: Fill height: Fit flow: Right spacing: 8. padding: 12.
                            hub_repo := TextInput{width: Fill height: 44 empty_text: "Dataset repository: owner/name"}
                            hub_go := Button{ text: "Transfer" height: 44 }
                            hub_cancel := Button{ text: "Cancel" height: 44 }
                        }
                        pages := View{
                            width: Fill height: Fill flow: Overlay
                            page_library := LibraryScreen{}
                            page_hardware := HardwareScreen{visible: false}
                            page_record := RecordScreen{visible: false}
                            page_play := PlayScreen{visible: false}
                            page_eval := EvalScreen{visible: false}
                            setup_page := View{
                                visible: false width: Fill height: Fill flow: Down padding: 24.0
                                setup_card := mod.widgets.ux.Card{
                                    width: Fill height: Fit flow: Down padding: 28.0 spacing: 18.0
                                    setup_title := Label{width: Fill text: "Set up your robot"
                                        draw_text +: {color: mod.theme.color_text text_style: mod.widgets.ux.TEXT_H1{}} }
                                    setup_description := Label{width: Fill height: Fit text: ""
                                        draw_text +: {color: mod.theme.color_text text_style: mod.widgets.ux.TEXT_BODY{}} }
                                    setup_actions := View{width: Fill height: Fit flow: Flow.Right{wrap: true} spacing: 12.0
                                        setup_rig := mod.widgets.nx.Primary{height: 44 text: "Choose rig profile…"}
                                        setup_rescan := mod.widgets.nx.NxBtn{height: 44 text: "Check connections"}
                                        setup_replay := mod.widgets.nx.NxBtn{height: 44 text: "Open replay"}
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

enum FolderRequest {
    Dataset,
    Download(String),
}

#[derive(Script, ScriptHook)]
pub struct App {
    #[live]
    ui: WidgetRef,
    /// Production backend by default; generated fixtures require explicit demo mode.
    #[rust(make_backend())]
    backend: Box<dyn Backend>,
    #[rust(false)]
    started: bool,
    #[rust]
    playback_timer: Timer,
    #[rust]
    last_tick: Option<std::time::Instant>,
    #[rust]
    last_refresh: Option<std::time::Instant>,
    #[rust]
    hub_upload: Option<bool>,
    #[rust]
    folder_request: Option<FolderRequest>,
}

impl App {
    fn save_session(&self) {
        if std::env::args().any(|a| a == "--demo") {
            return;
        }
        if let Some(path) = preferences::path() {
            let state = preferences::Session::snapshot(&*self.backend, frame::light_mode() < 0.5);
            if let Err(error) = preferences::save(&path, &state) {
                log::warn!("Could not save UI preferences: {}", error.kind());
            }
        }
    }
    /// Repaint every screen from the current backend snapshot.
    fn sync(&mut self, cx: &mut Cx) {
        self.ui.label(cx, ids!(status)).set_text(
            cx,
            self.backend
                .status()
                .unwrap_or("DEMO · generated fixtures; no hardware connected"),
        );
        self.ui
            .widget(cx, ids!(hub_form))
            .set_visible(cx, self.hub_upload.is_some());
        if self.started {
            self.save_session();
        }
        let light = frame::light_mode();
        dorobot_ux::sync_appearance(cx, light > 0.5);
        self.ui.button(cx, ids!(appearance)).set_text(
            cx,
            if light > 0.5 {
                "Appearance: Light"
            } else {
                "Appearance: Dark"
            },
        );
        self.ui.label(cx, ids!(page_title)).set_text(
            cx,
            match self.backend.screen() {
                Screen::Library => "Library",
                Screen::Record => "Record",
                Screen::Play => "Replay",
                Screen::Hardware => "Hardware",
                Screen::Eval => "Evaluate",
            },
        );
        frame::theme_chrome(cx, &self.ui, light);
        frame::apply_light(
            cx,
            &self.ui,
            &[
                (ids!(setup_card), frame::Themed::Bg),
                (ids!(setup_rescan), frame::Themed::Both),
                (ids!(setup_replay), frame::Themed::Both),
            ],
            light,
        );

        let nav = self.ui.widget(cx, ids!(nav));
        frame::sync_nav(cx, &nav, self.backend.screen());

        self.ui
            .library_screen(cx, ids!(page_library))
            .sync(cx, self.backend.library());
        self.ui
            .hardware_screen(cx, ids!(page_hardware))
            .sync(cx, self.backend.hardware());
        self.ui
            .record_screen(cx, ids!(page_record))
            .sync(cx, self.backend.record());
        self.ui
            .play_screen(cx, ids!(page_play))
            .sync(cx, self.backend.playback());
        self.ui
            .eval_screen(cx, ids!(page_eval))
            .sync(cx, self.backend.eval());

        // Overlay flow: exactly one page is visible at a time.
        let screen = self.backend.screen();
        let setup = match screen {
            Screen::Record => self.backend.record().profile_label.is_empty(),
            Screen::Hardware => self.backend.record().profile_label.is_empty(),
            Screen::Eval => self.backend.eval().checkpoint.is_empty(),
            Screen::Play => self.backend.playback().episodes.is_empty(),
            _ => false,
        };
        self.ui.widget(cx, ids!(setup_page)).set_visible(cx, setup);
        if setup {
            let evaluating = screen == Screen::Eval;
            let no_recordings = screen == Screen::Play;
            self.ui.button(cx, ids!(setup_replay)).set_text(
                cx,
                if no_recordings {
                    "Open library"
                } else {
                    "Open replay"
                },
            );
            self.ui.label(cx, ids!(setup_title)).set_text(
                cx,
                if no_recordings {
                    "Choose a recording to replay"
                } else if evaluating {
                    "No rollout running"
                } else {
                    "Connect a robot to get started"
                },
            );
            self.ui.label(cx, ids!(setup_description)).set_text(cx, if no_recordings {
                "Open a LeRobot dataset folder from Library. Your episodes, robot motion and camera recordings will appear here."
            } else if evaluating {
                "Live policy rollout is not configured. Use Nexus Studio to validate a checkpoint, or open Replay to inspect recorded motion."
            } else {
                "Choose a rig profile for your robot and cameras, then check the connections before recording. You can explore recordings in Replay without connecting hardware."
            });
            self.ui
                .widget(cx, ids!(setup_rig))
                .set_visible(cx, !evaluating && !no_recordings);
            self.ui
                .widget(cx, ids!(setup_rescan))
                .set_visible(cx, !evaluating && !no_recordings);
        }
        for (path, s) in [
            (ids!(page_library) as &[LiveId], Screen::Library),
            (ids!(page_hardware), Screen::Hardware),
            (ids!(page_record), Screen::Record),
            (ids!(page_play), Screen::Play),
            (ids!(page_eval), Screen::Eval),
        ] {
            self.ui
                .widget(cx, path)
                .set_visible(cx, s == screen && !setup);
        }
    }
}

impl MatchEvent for App {
    fn handle_startup(&mut self, cx: &mut Cx) {
        let args: Vec<String> = std::env::args().collect();
        let saved = if args.iter().any(|a| a == "--demo") {
            None
        } else {
            preferences::path().and_then(|p| preferences::load(&p))
        };
        if let Some(saved) = &saved {
            saved.restore(
                &mut *self.backend,
                !args.iter().any(|a| a == "--open" || a == "--dataset"),
            );
        }
        // Match the iOS sheet on first draw; --dark selects its dark appearance.
        frame::set_light_mode(
            if args.iter().any(|a| a == "--dark") || saved.as_ref().is_some_and(|s| s.dark) {
                0.0
            } else {
                1.0
            },
        );

        // Optional deep-link so the diff runner can shoot one screen per launch.
        let args: Vec<String> = std::env::args().collect();
        if let Some(i) = args.iter().position(|a| a == "--screen") {
            if let Some(name) = args.get(i + 1) {
                let target = match name.as_str() {
                    "record" => Some(Screen::Record),
                    "play" => Some(Screen::Play),
                    "hardware" => Some(Screen::Hardware),
                    "eval" => Some(Screen::Eval),
                    "library" => Some(Screen::Library),
                    _ => None,
                };
                if let Some(t) = target {
                    self.backend.dispatch(Intent::Navigate(t));
                }
            }
        }
        // --light lets the visual-diff runner shoot both themes.
        if args.iter().any(|a| a == "--light") {
            frame::set_light_mode(1.0);
        }
        self.sync(cx);
        self.started = true;
        self.playback_timer = cx.start_interval(1.0 / PLAYBACK_TIMER_FPS);
    }

    fn handle_timer(&mut self, cx: &mut Cx, event: &TimerEvent) {
        if self.playback_timer.is_timer(event).is_none() {
            return;
        }
        let now = std::time::Instant::now();
        let dt = self
            .last_tick
            .replace(now)
            .map(|t| now.duration_since(t).as_secs_f64())
            .unwrap_or(0.0);
        self.backend.poll();
        if self
            .last_refresh
            .is_none_or(|t| t.elapsed().as_millis() >= 100)
        {
            self.last_refresh = Some(now);
            self.ui.play_screen(cx, ids!(page_play)).adapt_layout(cx);
            if std::env::var_os("DOROBOT_UX_REPORT").is_some() {
                let play = self.backend.playback();
                dorobot_flex::ux_probe::write(
                    cx,
                    &self.ui,
                    serde_json::json!({
                        "screen": self.backend.screen().label(), "dark": frame::light_mode() < 0.5,
                        "episode": play.selected, "playing": play.is_playing, "time": play.current_time,
                        "frames": play.stats.frames, "tag": play.selected.and_then(|id| play.episodes.iter().find(|e| e.index == id)).map(|e| format!("{:?}", e.tag)),
                        "status": self.backend.status(), "hub_open": self.hub_upload.is_some(), "robot_animating": self.ui.robot_view(cx, ids!(stage_3d)).is_animating(), "signals_visible": self.ui.widget(cx, ids!(page_play.timeline.tl_body.lanes)).visible()
                    }),
                    &[
                        "nav_library",
                        "nav_record",
                        "nav_play",
                        "nav_hardware",
                        "nav_eval",
                        "appearance",
                        "setup_rig",
                        "setup_rescan",
                        "setup_replay",
                        "setup_page",
                        "episodes_btn",
                        "details_btn",
                        "signals_btn",
                        "signal_prev",
                        "signal_next",
                        "page_play.timeline.tl_body.lanes.content.lane_0",
                        "page_play.timeline.tl_body.lanes.content.lane_5",
                        "page_play.upper.tree.tree_body.row_0",
                        "page_play.upper.tree.tree_body.row_1",
                        "page_play.upper.tree.tree_body.row_2",
                        "page_play.upper.side.side_body.btn_good",
                        "page_play.upper.side.side_body.btn_bad",
                        "page_play.timeline.tl_body.controls.transport",
                        "play_btn",
                        "step_back_btn",
                        "step_fwd_btn",
                        "page_play.timeline.tl_body.tl",
                        "page_library.left_col.header.btn_pull",
                        "hub_repo",
                        "hub_cancel",
                        "page_play.upper.side",
                        "page_play.upper.tree",
                    ],
                );
            }
            self.ui
                .label(cx, ids!(status))
                .set_text(cx, self.backend.status().unwrap_or("DEMO"));
            match self.backend.screen() {
                Screen::Library => self
                    .ui
                    .library_screen(cx, ids!(page_library))
                    .sync(cx, self.backend.library()),
                Screen::Record => self
                    .ui
                    .record_screen(cx, ids!(page_record))
                    .sync(cx, self.backend.record()),
                Screen::Hardware => self
                    .ui
                    .hardware_screen(cx, ids!(page_hardware))
                    .sync(cx, self.backend.hardware()),
                _ => {}
            }
        }
        if !self.backend.playback().is_playing {
            return;
        }
        let st = self.backend.playback();
        let next = st.current_time + st.speed * dt;
        // Stop at the tail rather than wrapping: the last frame is a result
        // worth looking at.
        if next >= st.stats.duration_s {
            self.backend.dispatch(Intent::Seek(st.stats.duration_s));
            self.backend.dispatch(Intent::TogglePlay);
            self.sync(cx);
        } else {
            self.backend.dispatch(Intent::Seek(next));
            // Only the playhead moved, so only the views it drives are
            // touched; a full sync here does not fit in a 60Hz frame.
            self.ui
                .play_screen(cx, ids!(page_play))
                .tick(cx, self.backend.playback());
        }
        self.ui.redraw(cx);
    }

    fn handle_key_down(&mut self, cx: &mut Cx, e: &KeyEvent) {
        if e.modifiers.logo || e.modifiers.control {
            let target = match e.key_code {
                KeyCode::Key1 => Some(Screen::Library),
                KeyCode::Key2 => Some(Screen::Record),
                KeyCode::Key3 => Some(Screen::Play),
                KeyCode::Key4 => Some(Screen::Hardware),
                KeyCode::Key5 => Some(Screen::Eval),
                _ => None,
            };
            if let Some(target) = target {
                self.backend.dispatch(Intent::Navigate(target));
                self.sync(cx);
                return;
            }
        }
        if e.key_code == KeyCode::Escape && self.hub_upload.is_some() {
            self.hub_upload = None;
            self.sync(cx);
            return;
        }
        if self.hub_upload.is_some() {
            return;
        }

        let intent = if self.backend.screen() == Screen::Play {
            match e.key_code {
                KeyCode::Space => Some(Intent::TogglePlay),
                KeyCode::ArrowLeft => Some(Intent::StepFrames(-1)),
                KeyCode::ArrowRight => Some(Intent::StepFrames(1)),
                KeyCode::Home => Some(Intent::Seek(0.0)),
                KeyCode::End => Some(Intent::Seek(self.backend.playback().stats.duration_s)),
                KeyCode::KeyZ if e.modifiers.control || e.modifiers.logo => {
                    Some(Intent::UndoExclude)
                }
                _ => None,
            }
        } else {
            None
        };
        if let Some(intent) = intent {
            self.backend.dispatch(intent);
            self.sync(cx);
        }
    }

    fn handle_actions(&mut self, cx: &mut Cx, actions: &Actions) {
        let mut dirty = false;
        // The HAL opens Cocoa panels on its main thread and returns asynchronously.
        // Keep the original operation until either selection or cancellation arrives.
        for action in actions {
            if let Some(dialog) = action.downcast_ref::<FileDialogAction>() {
                match dialog {
                    FileDialogAction::FileSelected { id, paths }
                        if *id == live_id!(rig_profile) =>
                    {
                        if let Some(path) = paths.first() {
                            self.backend.dispatch(Intent::ConfigureRig(path.clone()));
                            self.backend.dispatch(Intent::NewRecordingSession);
                            dirty = true;
                        }
                    }
                    FileDialogAction::FolderSelected(path) => {
                        match self.folder_request.take() {
                            Some(FolderRequest::Dataset) => {
                                self.backend.dispatch(Intent::OpenFolder(path.clone()))
                            }
                            Some(FolderRequest::Download(repo)) => {
                                let name = repo.split('/').last().unwrap_or("dataset");
                                if !name.is_empty()
                                    && name != "."
                                    && name != ".."
                                    && !name.contains('\\')
                                {
                                    let destination = Some(path.join(name));
                                    self.backend
                                        .dispatch(Intent::HubTransfer { repo, destination });
                                    self.hub_upload = None;
                                }
                            }
                            None => {}
                        }
                        dirty = true;
                    }
                    FileDialogAction::FolderCancelled => {
                        self.folder_request = None;
                    }
                    _ => {}
                }
            }
        }
        for (id, step) in [(ids!(signal_prev), -1), (ids!(signal_next), 1)] {
            if self.ui.button(cx, id).clicked(actions) {
                self.ui
                    .play_screen(cx, ids!(page_play))
                    .shift_signals(cx, step);
            }
        }
        for (id, pane) in [
            (ids!(episodes_btn), "episodes"),
            (ids!(details_btn), "details"),
            (ids!(signals_btn), "signals"),
        ] {
            if self.ui.button(cx, id).clicked(actions) {
                self.ui
                    .play_screen(cx, ids!(page_play))
                    .toggle_pane(cx, pane);
            }
        }

        if self.ui.button(cx, ids!(setup_replay)).clicked(actions) {
            let target = if self.backend.screen() == Screen::Play {
                Screen::Library
            } else {
                Screen::Play
            };
            self.backend.dispatch(Intent::Navigate(target));
            dirty = true;
        }
        if self.ui.button(cx, ids!(setup_rescan)).clicked(actions) {
            self.backend.dispatch(Intent::WizardRestartStep);
            dirty = true;
        }
        let nav = self.ui.widget(cx, ids!(nav));
        for (path, screen) in frame::NAV_ITEMS {
            let item = nav.widget(cx, path);
            if item.is_empty() {
                continue;
            }
            if frame::view_clicked(actions, item.widget_uid()) {
                self.backend.dispatch(Intent::Navigate(screen));
                dirty = true;
            }
        }

        if self.ui.button(cx, ids!(appearance)).clicked(actions) {
            frame::toggle_light_mode();
            dirty = true;
        }

        if self
            .ui
            .button(cx, ids!(page_library.left_col.header.btn_new))
            .clicked(actions)
            || self.ui.button(cx, ids!(setup_rig)).clicked(actions)
        {
            cx.open_select_file_dialog(
                FileDialog::new()
                    .set_id(live_id!(rig_profile))
                    .set_title("Choose a rig profile".into())
                    .add_filter("Rig profile".into(), vec!["json".into()]),
            );
        }
        if self
            .ui
            .button(cx, ids!(page_library.left_col.header.btn_pull))
            .clicked(actions)
        {
            self.hub_upload = Some(false);
            dirty = true;
        }

        if self
            .ui
            .button(cx, ids!(page_library.left_col.header.btn_open))
            .clicked(actions)
        {
            if self.folder_request.is_none() {
                self.folder_request = Some(FolderRequest::Dataset);
                cx.open_select_folder_dialog(
                    FileDialog::new().set_title("Open LeRobot dataset or dataset library".into()),
                );
            }
        }
        for (path, intent) in [
            (
                ids!(page_record.transport.btn_record) as &[LiveId],
                if self.backend.record().recording {
                    Intent::RecordStop
                } else {
                    Intent::RecordStart
                },
            ),
            (ids!(page_record.transport.btn_save), Intent::SaveEpisode),
            (ids!(page_record.transport.btn_discard), Intent::DiscardLast),
            (ids!(page_record.transport.btn_rerecord), Intent::ReRecord),
        ] {
            if self.ui.button(cx, path).clicked(actions) {
                self.backend.dispatch(intent);
                dirty = true;
            }
        }

        if self
            .ui
            .button(cx, ids!(hub_form.hub_cancel))
            .clicked(actions)
        {
            self.hub_upload = None;
            dirty = true;
        }
        if self.ui.button(cx, ids!(hub_form.hub_go)).clicked(actions) {
            let repo = self
                .ui
                .widget(cx, ids!(hub_form.hub_repo))
                .text()
                .trim()
                .to_string();
            if !repo.is_empty() {
                if self.hub_upload == Some(true) {
                    self.backend.dispatch(Intent::HubTransfer {
                        repo,
                        destination: None,
                    });
                    self.hub_upload = None;
                } else if self.folder_request.is_none() {
                    self.folder_request = Some(FolderRequest::Download(repo));
                    cx.open_select_folder_dialog(
                        FileDialog::new().set_title("Choose download parent folder".into()),
                    );
                }
                dirty = true;
            }
        }

        // Hardware: the wizard's own buttons.
        if let Some(intent) = self
            .ui
            .hardware_screen(cx, ids!(page_hardware))
            .wizard_intent(cx, actions)
        {
            self.backend.dispatch(intent);
            dirty = true;
        }

        // Library: open the clicked dataset.
        let lib = self.ui.library_screen(cx, ids!(page_library));
        if lib.clicked_setup(cx, actions) {
            self.backend.dispatch(Intent::Navigate(Screen::Hardware));
            dirty = true;
        }
        if let Some(id) = lib.clicked_dataset(cx, actions, self.backend.library()) {
            self.backend.dispatch(Intent::OpenDataset(id));
            dirty = true;
        }

        // Transport: the Timeline widget owns scrub, play/pause and step, and
        // reports them as actions rather than mutating anything itself.
        for a in actions {
            if let Some(a) = a.as_widget_action() {
                match a.cast::<PlaybackAction>() {
                    PlaybackAction::Play | PlaybackAction::Pause => {
                        self.backend.dispatch(Intent::TogglePlay);
                        dirty = true;
                    }
                    PlaybackAction::StepForward => {
                        self.backend.dispatch(Intent::StepFrames(1));
                        dirty = true;
                    }
                    PlaybackAction::StepBackward => {
                        self.backend.dispatch(Intent::StepFrames(-1));
                        dirty = true;
                    }
                    PlaybackAction::None => {}
                }
                match a.cast::<TimelineAction>() {
                    TimelineAction::Play | TimelineAction::Pause => {
                        self.backend.dispatch(Intent::TogglePlay);
                        dirty = true;
                    }
                    TimelineAction::Seek(t) => {
                        self.backend.dispatch(Intent::Seek(t));
                        dirty = true;
                    }
                    TimelineAction::StepForward => {
                        self.backend.dispatch(Intent::StepFrames(1));
                        dirty = true;
                    }
                    TimelineAction::StepBackward => {
                        self.backend.dispatch(Intent::StepFrames(-1));
                        dirty = true;
                    }
                    TimelineAction::SpeedChanged(v) => {
                        self.backend.dispatch(Intent::SetSpeed(v));
                        dirty = true;
                    }
                    TimelineAction::ScrubEnd | TimelineAction::None => {}
                }
                // Dragging the plot cursor scrubs too, so the trace and the
                // video stay one control.
                if let TimeSeriesPlotAction::CursorMoved(t) = a.cast::<TimeSeriesPlotAction>() {
                    self.backend.dispatch(Intent::Seek(t));
                    dirty = true;
                }
            }
        }

        // Play: episode selection and curation.
        let play = self.ui.play_screen(cx, ids!(page_play));
        if let Some(ep) = play.clicked_episode(cx, actions) {
            self.backend.dispatch(Intent::SelectEpisode(ep));
            dirty = true;
        }
        if let Some(sel) = self.backend.playback().selected {
            if let Some(intent) = play.curation_intent(cx, actions, sel) {
                if matches!(intent, Intent::PushToHub) {
                    self.hub_upload = Some(true);
                } else {
                    self.backend.dispatch(intent);
                }
                dirty = true;
            }
        }

        if dirty {
            self.sync(cx);
            self.ui.redraw(cx);
        }
    }
}

impl AppMain for App {
    fn script_mod(vm: &mut ScriptVm) -> ScriptValue {
        dorobot_ux::install_ios(vm, !std::env::args().any(|a| a == "--dark"));
        makepad_widgets::script_mod(vm);
        // Shared tokens first: flex's styles alias onto them.
        dorobot_ux::script_mod(vm);
        // RobotView, and the XR scene it draws into, back the Play 3D pane
        makepad_urdf_player::makepad_xr::script_mod(vm);
        makepad_urdf_player::script_mod(vm);
        // app-shell supplies the real draggable PanelGrid used by the Play screen
        dorobot_flex::makepad_app_shell::script_mod_with_theme(vm, |vm| {
            dorobot_flex::shared::shell_theme::script_mod(vm);
        });
        // TimeSeriesPlot lives in widgets and is used by the Play screen's plot
        // pane, so both must register before ui::script_mod evaluates that DSL.
        // shared goes first: it defines the mod.widgets.flex text styles the
        // widgets module builds on.
        dorobot_flex::shared::script_mod(vm);
        dorobot_flex::widgets::script_mod(vm);
        dorobot_flex::playback_controls::script_mod(vm);
        dorobot_flex::ui::script_mod(vm);
        self::script_mod(vm)
    }

    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        if matches!(event, Event::Shutdown | Event::WindowClosed(_)) {
            self.save_session();
        }
        self.match_event(cx, event);
        self.ui.handle_event(cx, event, &mut Scope::empty());
        if matches!(event, Event::LiveEdit | Event::ScriptReapply) && self.started {
            self.ui
                .play_screen(cx, ids!(page_play))
                .restore_panel_design(cx);
            self.sync(cx);
        }
        // Scrolling the episode window produces no action, so it is picked up
        // here rather than in handle_actions, which would not run for it.
        if self.ui.play_screen(cx, ids!(page_play)).take_scrolled() {
            self.sync(cx);
            self.ui.redraw(cx);
        }
    }
}
