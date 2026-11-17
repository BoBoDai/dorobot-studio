//! Studio 宿主与共享服务的兼容入口。
pub use dorobot_studio_core::*;
pub mod plugins;
pub mod ui {
    pub use dorobot_plugin_collection::screen as record;
    pub use dorobot_plugin_control::screen as hardware;
    pub use dorobot_studio_core::ui::*;
    pub fn script_mod(vm: &mut makepad_widgets::ScriptVm) -> makepad_widgets::ScriptValue {
        dorobot_studio_core::ui::script_mod(vm);
        dorobot_plugin_control::screen::script_mod(vm);
        dorobot_plugin_collection::screen::script_mod(vm)
    }
}
