// DoRobot Studio: real datasets and LeRobot capture, with explicit --demo fixtures.

use dorobot_flex::api::{files::FileBackend, mock::MockBackend, Backend, Intent, Screen};
use dorobot_flex::makepad_app_shell::shell::layout::ShellLayoutWidgetRefExt;
use dorobot_flex::makepad_app_shell::shell::header::ShellHeaderAction;
use dorobot_flex::playback_controls::PlaybackAction;
use dorobot_flex::ui::{
    eval::EvalScreenWidgetRefExt, frame, hardware::HardwareScreenWidgetRefExt,
    library::LibraryScreenWidgetRefExt, play::PlayScreenWidgetRefExt,
    record::RecordScreenWidgetRefExt,
};
use dorobot_flex::widgets::time_series_plot::TimeSeriesPlotAction;
use dorobot_flex::widgets::timeline::TimelineAction;
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
                    flow: Down
                    width: Fill
                    height: Fill
                    show_bg: true
                    draw_bg.color: mod.theme.color_bg_app

                    hub_form := View{ visible: false width: Fill height: 42 flow: Right spacing: 8.
                        Label{ text: "Dataset repo (owner/name):" }
                        hub_repo := TextInput{ width: Fill height: 36 }
                        hub_go := Button{ text: "Transfer" }
                        hub_cancel := Button{ text: "Cancel" }
                    }
                    status := Label{ width: Fill height: Fit text: "" draw_text +: { color: mod.theme.color_text text_style: mod.widgets.ux.TEXT_META{} } }
                    // The screens sit in app-shell's ShellLayout rather than the
                    // hand-rolled AppBar + work-row they were prototyped in, so
                    // they inherit what the framework provides: a collapsible
                    // rail, resizable regions, and panels that dock and persist.
                    //
                    // The nav rail and the pages keep their ids, so the screen
                    // switching in `Screen::apply` still resolves.
                    shell := ShellLayout{
                        width: Fill
                        height: Fill

                        // No floating menu. app-shell ships a hover/pinned
                        // overlay sidebar (stock "Quick Actions"), but this app
                        // already has a permanent nav rail — a second menu over
                        // the top of it is redundant. Both presentations are
                        // hidden, and the hamburger with them, since it would
                        // otherwise be a control that does nothing.
                        pinned_sidebar +: { visible: false width: 0 }
                        overlay_sidebar +: { visible: false width: 0 }

                        main_container +: {
                            header +: {
                                hamburger_btn +: { visible: false width: 0 }
                                title_label +: { text: "DoRobot Studio" }
                            }

                            dock_wrapper +: {
                                dock +: {
                                    // Region sizes. The shell defaults suit a
                                    // three-pane IDE; this UX wants a narrow
                                    // icon rail, no footer strip, and no right
                                    // rail — its screens bring their own side
                                    // content.
                                    root +: { align: SplitterAlign.FromB(0.0) }
                                    main_area +: { align: SplitterAlign.FromA(96.0) }
                                    right_area +: { align: SplitterAlign.FromB(0.0) }

                                    // The shell's footer grid ships seven empty
                                    // demo panels; nothing here uses it.
                                    footer_content: View{
                                        width: Fill
                                        height: Fill
                                    }

                                    left_sidebar_content: View{
                                        width: Fill
                                        height: Fill
                                        nav := mod.widgets.ux.NavRail{
                                            width: Fill
                                            height: Fill
                                        }
                                    }

                                    center_content: View{
                                        width: Fill
                                        height: Fill
                                        pages := View{
                                            width: Fill
                                            height: Fill
                                            flow: Overlay
                                            page_library := LibraryScreen{}
                                            page_hardware := HardwareScreen{ visible: false }
                                            page_record := RecordScreen{ visible: false }
                                            page_play := PlayScreen{ visible: false }
                                            page_eval := EvalScreen{ visible: false }
                                        }
                                    }

                                    // The screens carry their own side content
                                    // (Library's Hardware panel, Play's inspector),
                                    // so the shell's own rails stay empty.
                                    right_sidebar_content: View{
                                        width: Fill
                                        height: Fill
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
}

impl App {
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
        let light = frame::light_mode();
        dorobot_ux::sync_appearance(cx, light > 0.5);
        self.ui.shell_layout(cx, ids!(shell)).set_dark_mode(cx, light < 0.5);
        frame::theme_chrome(cx, &self.ui, light);

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
        for (path, s) in [
            (ids!(page_library) as &[LiveId], Screen::Library),
            (ids!(page_hardware), Screen::Hardware),
            (ids!(page_record), Screen::Record),
            (ids!(page_play), Screen::Play),
            (ids!(page_eval), Screen::Eval),
        ] {
            self.ui.widget(cx, path).set_visible(cx, s == screen);
        }
    }
}

impl MatchEvent for App {
    fn handle_startup(&mut self, cx: &mut Cx) {
        // Match the iOS sheet on first draw; --dark selects its dark appearance.
        frame::set_light_mode(if std::env::args().any(|a| a == "--dark") { 0.0 } else { 1.0 });

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

        // The shell header captures its button actions and emits this action.
        let header = self.ui.widget(cx, ids!(shell.main_container.header));
        if actions.filter_widget_actions(header.widget_uid()).any(|action| {
            matches!(action.cast::<ShellHeaderAction>(), ShellHeaderAction::ToggleDarkMode)
        }) {
            frame::toggle_light_mode();
            dirty = true;
        }

        if self
            .ui
            .button(cx, ids!(page_library.left_col.header.btn_new))
            .clicked(actions)
        {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Choose a rig profile (tools/capture/rig.example.json)")
                .add_filter("Rig profile", &["json"])
                .pick_file()
            {
                self.backend.dispatch(Intent::ConfigureRig(path));
                self.backend.dispatch(Intent::NewRecordingSession);
                dirty = true;
            }
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
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Open LeRobot dataset or dataset library")
                .pick_folder()
            {
                self.backend.dispatch(Intent::OpenFolder(path));
                dirty = true;
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
                } else if let Some(parent) = rfd::FileDialog::new()
                    .set_title("Choose download parent folder")
                    .pick_folder()
                {
                    let name = repo.split('/').last().unwrap_or("dataset");
                    if name != "." && name != ".." && !name.contains('\\') {
                        let destination = Some(parent.join(name));
                        self.backend
                            .dispatch(Intent::HubTransfer { repo, destination });
                        self.hub_upload = None;
                    }
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
        self.match_event(cx, event);
        self.ui.handle_event(cx, event, &mut Scope::empty());
        if matches!(event, Event::LiveEdit | Event::ScriptReapply) && self.started {
            self.ui.play_screen(cx, ids!(page_play)).restore_panel_design(cx);
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
