use dorobot_flex::api::{mock::MockBackend, Backend, Intent};
use dorobot_studio_plugin::*;

fn setup() -> (Cx, PluginRegistry) {
    let mut cx = Cx::new(Box::new(|_, _| {}));
    cx.init_cx_os();
    let registry = dorobot_flex::plugins::builtin_plugins();
    cx.with_vm(|vm| {
        dorobot_studio_core::register_ui(vm);
        registry.register_widgets(vm);
        assert!(vm.take_errors().is_empty());
    });
    (cx, registry)
}

#[test]
fn collection_factory_creates_independent_real_views_and_routes_record_stop_save() {
    let (mut cx, registry) = setup();
    let mut first = cx
        .with_vm(|vm| registry.create_instance("page_record", vm))
        .unwrap();
    let second = cx
        .with_vm(|vm| registry.create_instance("page_record", vm))
        .unwrap();
    assert_ne!(first.root().widget_uid(), second.root().widget_uid());
    let mut backend = MockBackend::new();
    backend.dispatch(Intent::DiscardLast);
    if backend.record().recording {
        backend.dispatch(Intent::RecordStop);
        backend.dispatch(Intent::DiscardLast);
    }
    for (path, recording, saved_delta) in [
        (ids!(transport.btn_record) as &[LiveId], true, 0),
        (ids!(transport.btn_record), false, 0),
        (ids!(transport.btn_save), false, 1),
    ] {
        let previous = backend.record().saved;
        let button = first.root().button(&mut cx, path);
        assert!(!button.is_empty());
        let actions = cx.capture_actions(|cx| {
            cx.widget_action(
                button.widget_uid(),
                ButtonAction::Clicked(KeyModifiers::default()),
            )
        });
        assert!(first.handle_actions(&mut cx, &actions, &mut backend));
        assert_eq!(backend.record().recording, recording);
        assert_eq!(backend.record().saved, previous + saved_delta);
    }
}

#[test]
fn collection_cannot_be_disabled_with_unsaved_take() {
    let mut registry = dorobot_flex::plugins::builtin_plugins();
    let mut backend = MockBackend::new();
    backend.dispatch(Intent::RecordStart);
    assert!(registry
        .set_enabled("dorobot.collection", false, &backend)
        .is_err());
    backend.dispatch(Intent::RecordStop);
    assert!(registry
        .set_enabled("dorobot.collection", false, &backend)
        .is_err());
    backend.dispatch(Intent::SaveEpisode);
    registry
        .set_enabled("dorobot.collection", false, &backend)
        .unwrap();
    assert!(!registry.enabled("dorobot.collection"));
    assert!(registry.enabled("dorobot.control"));
}

#[test]
fn control_factory_preserves_the_hardware_wizard_action() {
    let (mut cx, registry) = setup();
    let mut panel = cx
        .with_vm(|vm| registry.create_instance("page_hardware", vm))
        .unwrap();
    let mut backend = MockBackend::new();
    let before = backend
        .hardware()
        .steps
        .iter()
        .position(|s| s.state == dorobot_flex::api::StepState::Active)
        .unwrap();
    let button = panel.root().button(
        &mut cx,
        ids!(content.camera_a_panel.content.calib.actions.btn_continue),
    );
    assert!(!button.is_empty());
    let actions = cx.capture_actions(|cx| {
        cx.widget_action(
            button.widget_uid(),
            ButtonAction::Clicked(KeyModifiers::default()),
        )
    });
    assert!(panel.handle_actions(&mut cx, &actions, &mut backend));
    assert_eq!(
        backend.hardware().steps[before].state,
        dorobot_flex::api::StepState::Done
    );
}

struct ExtraPlugin;
impl StudioPlugin for ExtraPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: "independent.example",
            title: "Example",
            api_version: API_VERSION,
            panels: &[PanelDescriptor {
                id: "independent_panel",
                title: "Independent panel",
                legacy_screen: None,
            }],
        }
    }
    fn register(&self, _: &mut ScriptVm) {}
    fn create(&self, _: &str, vm: &mut ScriptVm) -> Result<Box<dyn PluginPanel>, String> {
        let value = script_eval!(vm, { use mod.prelude.widgets.* use mod.widgets.* View{width: Fill height: Fill} });
        Ok(Box::new(ExtraPanel {
            root: WidgetRef::script_from_value(vm, value),
        }))
    }
}
struct ExtraPanel {
    root: WidgetRef,
}
impl PluginPanel for ExtraPanel {
    fn root(&self) -> WidgetRef {
        self.root.clone()
    }
    fn sync(&mut self, _: &mut Cx, _: &PanelContext<'_>) {}
    fn handle_actions(&mut self, _: &mut Cx, _: &Actions, _: &mut dyn Backend) -> bool {
        false
    }
}

#[test]
fn independent_panel_mounts_without_a_new_screen_variant_and_reuses_its_instance() {
    let (mut cx, mut registry) = setup();
    registry.register(Box::new(ExtraPlugin)).unwrap();
    let container = cx.with_vm(|vm| {
        let value = script_eval!(vm, { use mod.prelude.widgets.* use mod.widgets.* View{} });
        WidgetRef::script_from_value(vm, value)
    });
    registry.mount(&mut cx, &container).unwrap();
    let first = container.widget(&cx, ids!(independent_panel));
    assert!(!first.is_empty());
    registry.mount(&mut cx, &container).unwrap();
    assert_eq!(
        first.widget_uid(),
        container.widget(&cx, ids!(independent_panel)).widget_uid()
    );
    let backend = MockBackend::new();
    registry.select("independent_panel").unwrap();
    registry.sync(
        &mut cx,
        &PanelContext {
            backend: &backend,
            rig_editing: false,
            show_help: false,
            show_details: false,
            live_controls: false,
        },
    );
    assert!(first.visible());
    assert!(!container.widget(&cx, ids!(page_record)).visible());
    registry
        .set_enabled("independent.example", false, &backend)
        .unwrap();
    registry.sync(
        &mut cx,
        &PanelContext {
            backend: &backend,
            rig_editing: false,
            show_help: false,
            show_details: false,
            live_controls: false,
        },
    );
    assert!(!first.visible());
}

#[test]
fn every_builtin_factory_and_toolbox_has_no_script_errors() {
    let (mut cx, mut registry) = setup();
    for descriptor in registry.descriptors() {
        for panel in descriptor.panels {
            cx.with_vm(|vm| {
                registry.create_instance(panel.id, vm).unwrap();
                let errors = vm.take_errors();
                assert!(errors.is_empty(), "{}: {errors:?}", panel.id);
            });
        }
    }
    let container = cx.with_vm(|vm| {
        let value = script_eval!(vm, { use mod.prelude.widgets.* use mod.prelude.widgets.* use mod.widgets.* View{} });
        WidgetRef::script_from_value(vm, value)
    });
    registry.mount_toolbox(&mut cx, &container).unwrap();
    let tabs = cx.with_vm(|vm| {
        let value = script_eval!(vm, { use mod.prelude.widgets.* use mod.widgets.* View{} });
        WidgetRef::script_from_value(vm, value)
    });
    registry.mount_tabs(&mut cx, &tabs).unwrap();
    registry.sync_navigation(&mut cx, dorobot_flex::api::Screen::Hardware);
    cx.with_vm(|vm| {
        let errors = vm.take_errors();
        assert!(errors.is_empty(), "{errors:?}");
    });
}

#[test]
fn toolbox_has_one_entry_per_plugin_instead_of_one_per_panel() {
    let (mut cx, mut registry) = setup();
    let container = cx.with_vm(|vm| {
        let value = script_eval!(vm, { use mod.prelude.widgets.* use mod.widgets.* View{} });
        WidgetRef::script_from_value(vm, value)
    });
    registry.mount_toolbox(&mut cx, &container).unwrap();
    let children = container.borrow::<View>().unwrap().children.clone();
    assert_eq!(children.len(), 2, "侧栏应按插件分组，不逐个展开面板");
    assert_eq!(children[0].1.text(), "Robot control");
    assert_eq!(children[1].1.text(), "Data collection");
}

#[test]
fn plugin_tabs_follow_navigation_and_group_entries_restore_the_last_tab() {
    use dorobot_flex::api::Screen;
    let (mut cx, mut registry) = setup();
    let root = cx.with_vm(|vm| {
        let value = script_eval!(vm, { use mod.prelude.widgets.* use mod.widgets.* View{toolbox := View{} tabs := View{}} });
        WidgetRef::script_from_value(vm, value)
    });
    let toolbox = root.widget(&cx, ids!(toolbox));
    let tabs = root.widget(&cx, ids!(tabs));
    registry.mount_toolbox(&mut cx, &toolbox).unwrap();
    registry.mount_tabs(&mut cx, &tabs).unwrap();
    let setup = root.widget(&cx, ids!(plugin_tab_page_hardware));
    let operate = root.widget(&cx, ids!(plugin_tab_hardware_teleop));
    let record = root.widget(&cx, ids!(plugin_tab_page_record));
    registry.sync_navigation(&mut cx, Screen::Hardware);
    assert!(setup.visible() && operate.visible());
    assert!(!record.visible());
    let operate_click = cx.capture_actions(|cx| {
        cx.widget_action(
            operate.widget_uid(),
            ButtonAction::Clicked(KeyModifiers::default()),
        )
    });
    assert_eq!(
        registry
            .tab_selection(&operate_click)
            .unwrap()
            .legacy_screen,
        Some(Screen::Live)
    );
    registry.sync_navigation(&mut cx, Screen::Live);
    registry.sync_navigation(&mut cx, Screen::Record);
    assert!(!setup.visible() && !operate.visible());
    assert!(record.visible());
    assert!(
        registry.tab_selection(&operate_click).is_none(),
        "隐藏标签不能触发跳转"
    );
    let control = root.widget(&cx, &[LiveId::from_str("toolbox_dorobot.control")]);
    let group_click = cx.capture_actions(|cx| {
        cx.widget_action(
            control.widget_uid(),
            ButtonAction::Clicked(KeyModifiers::default()),
        )
    });
    assert_eq!(
        registry
            .toolbox_selection(&mut cx, &group_click)
            .unwrap()
            .id,
        "hardware_teleop"
    );
    registry.sync_navigation(&mut cx, Screen::Play);
    assert!(!setup.visible() && !operate.visible() && !record.visible());
    cx.with_vm(|vm| assert!(vm.take_errors().is_empty()));
}

#[test]
fn independent_plugin_navigation_survives_theme_reapply_and_blocks_disabled_entries() {
    use dorobot_flex::api::Screen;
    let (mut cx, mut registry) = setup();
    registry.register(Box::new(ExtraPlugin)).unwrap();
    let root = cx.with_vm(|vm| {
        let value = script_eval!(vm, { use mod.prelude.widgets.* use mod.widgets.* View{toolbox := View{} tabs := View{}} });
        WidgetRef::script_from_value(vm, value)
    });
    let toolbox = root.widget(&cx, ids!(toolbox));
    let tabs = root.widget(&cx, ids!(tabs));
    registry.mount_toolbox(&mut cx, &toolbox).unwrap();
    registry.mount_tabs(&mut cx, &tabs).unwrap();
    registry.select("independent_panel").unwrap();
    registry.sync_navigation(&mut cx, Screen::Play);
    assert_eq!(
        registry.active_plugin(Screen::Play).unwrap().id,
        "independent.example"
    );
    let tab = root.widget(&cx, ids!(plugin_tab_independent_panel));
    assert!(tab.visible());
    registry.reapply(&mut cx);
    toolbox.borrow_mut::<View>().unwrap().children.clear();
    tabs.borrow_mut::<View>().unwrap().children.clear();
    registry.mount_toolbox(&mut cx, &toolbox).unwrap();
    registry.mount_tabs(&mut cx, &tabs).unwrap();
    registry.sync_navigation(&mut cx, Screen::Play);
    assert_eq!(
        root.widget(&cx, ids!(plugin_tab_independent_panel))
            .widget_uid(),
        tab.widget_uid()
    );
    assert!(tab.visible());
    let group = root.widget(&cx, &[LiveId::from_str("toolbox_independent.example")]);
    let click = cx.capture_actions(|cx| {
        cx.widget_action(
            group.widget_uid(),
            ButtonAction::Clicked(KeyModifiers::default()),
        )
    });
    assert_eq!(
        registry.toolbox_selection(&mut cx, &click).unwrap().id,
        "independent_panel"
    );
    registry
        .set_enabled("independent.example", false, &MockBackend::new())
        .unwrap();
    registry.sync_navigation(&mut cx, Screen::Play);
    assert!(!tab.visible());
    assert!(registry.toolbox_selection(&mut cx, &click).is_none());
    cx.with_vm(|vm| assert!(vm.take_errors().is_empty()));
}

#[test]
fn plugin_panel_named_play_cannot_shadow_the_hosts_replay_tab() {
    struct NamedPlay;
    impl StudioPlugin for NamedPlay {
        fn descriptor(&self) -> PluginDescriptor {
            PluginDescriptor {
                id: "example.play",
                title: "Example",
                api_version: API_VERSION,
                panels: &[PanelDescriptor {
                    id: "play",
                    title: "Example play",
                    legacy_screen: None,
                }],
            }
        }
        fn register(&self, _: &mut ScriptVm) {}
        fn create(&self, panel: &str, vm: &mut ScriptVm) -> Result<Box<dyn PluginPanel>, String> {
            ExtraPlugin.create(panel, vm)
        }
    }
    let (mut cx, mut registry) = setup();
    registry.register(Box::new(NamedPlay)).unwrap();
    let root = cx.with_vm(|vm| {
        let value = script_eval!(vm, {
            use mod.prelude.widgets.* use mod.widgets.*
            View{plugin_tabs := View{} data_tabs := View{tab_play := mod.widgets.ux.WorkspaceTab{text: "Replay"}}}
        });
        WidgetRef::script_from_value(vm, value)
    });
    let tabs = root.widget(&cx, ids!(plugin_tabs));
    registry.mount_tabs(&mut cx, &tabs).unwrap();
    let replay = root.widget(&cx, ids!(data_tabs.tab_play));
    assert_eq!(
        root.widget(&cx, ids!(tab_play)).widget_uid(),
        replay.widget_uid()
    );
}

#[test]
fn toolbox_controls_cannot_shadow_panel_paths() {
    let (mut cx, mut registry) = setup();
    let root = cx.with_vm(|vm| {
        let value = script_eval!(vm, { use mod.prelude.widgets.* use mod.widgets.* View{ toolbox := View{} pages := View{} } });
        WidgetRef::script_from_value(vm, value)
    });
    let toolbox = root.widget(&cx, ids!(toolbox));
    let pages = root.widget(&cx, ids!(pages));
    registry.mount_toolbox(&mut cx, &toolbox).unwrap();
    registry.mount(&mut cx, &pages).unwrap();
    assert!(root
        .widget(&cx, ids!(page_hardware))
        .borrow::<dorobot_plugin_control::screen::HardwareScreen>()
        .is_some());
    assert!(root
        .widget(&cx, ids!(page_record))
        .borrow::<dorobot_plugin_collection::screen::RecordScreen>()
        .is_some());
}

#[test]
fn remount_after_container_reapply_restores_the_same_panel_instances() {
    let (mut cx, mut registry) = setup();
    let container = cx.with_vm(|vm| {
        let value = script_eval!(vm, { use mod.prelude.widgets.* use mod.widgets.* View{} });
        WidgetRef::script_from_value(vm, value)
    });
    registry.mount(&mut cx, &container).unwrap();
    let original = container.widget(&cx, ids!(page_record));
    container.borrow_mut::<View>().unwrap().children.clear();
    cx.widget_tree_mark_dirty(container.widget_uid());
    registry.mount(&mut cx, &container).unwrap();
    assert_eq!(
        container.widget(&cx, ids!(page_record)).widget_uid(),
        original.widget_uid()
    );
}

#[test]
fn disconnected_operate_panel_respects_the_hosts_setup_gate() {
    let (mut cx, registry) = setup();
    let mut panel = cx
        .with_vm(|vm| registry.create_instance("hardware_teleop", vm))
        .unwrap();
    let root = std::env::temp_dir().join(format!("dorobot-plugin-empty-{}", std::process::id()));
    let mut backend = dorobot_flex::api::files::FileBackend::new(root);
    backend.dispatch(Intent::Navigate(dorobot_flex::api::Screen::Live));
    assert!(backend.playback().live.is_some());
    let context = PanelContext {
        backend: &backend,
        rig_editing: false,
        show_help: false,
        show_details: false,
        live_controls: false,
    };
    assert_eq!(panel.visible_in(&context), Some(false));
    panel.sync(&mut cx, &context);
}
