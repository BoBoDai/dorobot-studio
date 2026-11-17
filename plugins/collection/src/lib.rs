//! 数据采集插件：拥有真实采集界面和界面到业务服务的命令映射。
pub mod screen;
use dorobot_studio_core::api::{Backend, Intent, Screen};
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
        }))
    }
    fn can_disable(&self, backend: &dyn Backend) -> Result<(), String> {
        let state = backend.record();
        if state.recording
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
}
impl PluginPanel for CollectionPanel {
    fn root(&self) -> WidgetRef {
        self.root.clone()
    }
    fn visible_in(&self, context: &PanelContext<'_>) -> Option<bool> {
        Some(
            context.backend.screen() == Screen::Record
                && !context.backend.workflow().simulation
                && context.backend.hardware().nano.is_none()
                && context.backend.playback().live.is_none(),
        )
    }
    fn sync(&mut self, cx: &mut Cx, context: &PanelContext<'_>) {
        self.root
            .as_record_screen()
            .sync(cx, context.backend.record());
    }
    fn handle_actions(
        &mut self,
        cx: &mut Cx,
        actions: &Actions,
        backend: &mut dyn Backend,
    ) -> bool {
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
