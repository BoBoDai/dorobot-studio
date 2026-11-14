//! Stable panel identities: changing geometry never swaps media or widget state.
use makepad_widgets::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelLayout {
    pub order: Vec<usize>,
    pub visible: [bool; 4],
    pub maximized: Option<usize>,
    pub columns: usize,
    pub split_x: f64,
    pub split_y: f64,
}
impl Default for PanelLayout {
    fn default() -> Self {
        Self {
            order: vec![0, 1, 2, 3],
            visible: [true; 4],
            maximized: None,
            columns: 2,
            split_x: 0.5,
            split_y: 0.5,
        }
    }
}
impl PanelLayout {
    pub fn validate(&self) -> bool {
        let mut order = self.order.clone();
        order.sort_unstable();
        order == [0, 1, 2, 3]
            && (1..=4).contains(&self.columns)
            && self.maximized.is_none_or(|i| i < 4 && self.visible[i])
            && self.split_x.is_finite()
            && (0.2..=0.8).contains(&self.split_x)
            && self.split_y.is_finite()
            && (0.2..=0.8).contains(&self.split_y)
    }
    fn path(name: &str) -> Option<std::path::PathBuf> {
        crate::preferences::path().map(|p| p.with_file_name(name))
    }
    fn load(name: &str) -> Self {
        Self::path(name)
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Self>(&b).ok())
            .filter(Self::validate)
            .unwrap_or_default()
    }
    fn save(&self, name: &str) {
        if let Some(path) = Self::path(name) {
            let result = (|| -> std::io::Result<()> {
                std::fs::create_dir_all(path.parent().unwrap())?;
                let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
                std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
                std::fs::rename(tmp, path)
            })();
            if let Err(e) = result {
                ::log::warn!("Could not save replay layout: {}", e.kind());
            }
        }
    }
    pub fn move_before(&mut self, source: usize, target: usize) {
        if source == target || source >= 4 || target >= 4 {
            return;
        }
        self.order.retain(|i| *i != source);
        let at = self
            .order
            .iter()
            .position(|i| *i == target)
            .unwrap_or(self.order.len());
        self.order.insert(at, source);
    }
    pub fn toggle(&mut self, id: usize) {
        if id >= 4 {
            return;
        }
        self.visible[id] = !self.visible[id];
        self.maximized = None;
    }
}

script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*
    mod.widgets.FlexPanelsBase = #(FlexPanels::register_widget(vm))
    mod.widgets.FlexPanels = set_type_default() do mod.widgets.FlexPanelsBase{
        persistence_key: "replay-layout.json"
        panel_count: 4
        width: Fill height: Fill flow: Overlay
        show_bg: false
    }
    // Desktop panel utilities should not inherit the iOS 44px touch minimum.
    mod.widgets.PanelToolButton = mod.widgets.ux.ToolButton{
        width: 28 height: 28 min_height: 28
        padding: 0.0
    }
    mod.widgets.StudioPanel = mod.widgets.ux.Card{
        width: Fill height: Fill flow: Down
        head := View{width: Fill height: 40 flow: Right spacing: 4.0 align: Align{y: 0.5}
            cursor: MouseCursor.Hand
            grip := Label{width: 18 text: "⠿" draw_text.color: mod.theme.color_text_disabled}
            title := Label{width: Fill max_lines: 1 text_overflow: Ellipsis
                draw_text +: {color: mod.theme.color_text text_style: mod.widgets.ux.TEXT_NAV{}}}
            maximize := mod.widgets.PanelToolButton{text: "⤢"}
            close := mod.widgets.PanelToolButton{text: "×"}
        }
    }
}

#[derive(Script, ScriptHook, Widget)]
pub struct FlexPanels {
    #[deref]
    view: View,
    #[live]
    persistence_key: String,
    #[live(4usize)]
    panel_count: usize,
    #[rust]
    loaded: bool,
    #[rust(-1.0)]
    themed_light: f64,
    #[rust]
    state: PanelLayout,
    #[rust]
    drag: Option<(usize, DVec2)>,
    #[rust]
    resize: Option<bool>,
    #[rust]
    drop_target: Option<usize>,
    #[rust]
    rect: Rect,
    #[rust]
    panel_rects: [Rect; 4],
    #[rust]
    handles: [Rect; 2],
    #[live]
    preview: DrawColor,
}
const PANELS: [&[LiveId]; 4] = [
    ids!(robot_panel),
    ids!(camera_a_panel),
    ids!(camera_b_panel),
    ids!(error_panel),
];

impl FlexPanels {
    fn changed(&mut self, cx: &mut Cx) {
        self.state.save(&self.persistence_key);
        self.view.redraw(cx);
    }
    fn place(&mut self, cx: &mut Cx, rect: Rect) {
        self.rect = rect;
        self.panel_rects = [Rect::default(); 4];
        self.handles = [Rect::default(); 2];
        let visible: Vec<_> = self
            .state
            .order
            .iter()
            .copied()
            .filter(|i| self.state.visible[*i] && self.state.maximized.is_none_or(|m| m == *i))
            .collect();
        let cols = self.state.columns.min(visible.len()).max(1);
        let rows = visible.len().div_ceil(cols).max(1);
        let gap = 8.0;
        for (slot, id) in visible.iter().copied().enumerate() {
            let (col, row) = (slot % cols, slot / cols);
            let (x, w) = if cols == 2 {
                let left = (rect.size.x - gap).max(0.0) * self.state.split_x;
                if col == 0 {
                    (0.0, left)
                } else {
                    (left + gap, rect.size.x - left - gap)
                }
            } else {
                let w = (rect.size.x - gap * (cols - 1) as f64) / cols as f64;
                (col as f64 * (w + gap), w)
            };
            let (y, h) = if rows == 2 {
                let top = (rect.size.y - gap).max(0.0) * self.state.split_y;
                if row == 0 {
                    (0.0, top)
                } else {
                    (top + gap, rect.size.y - top - gap)
                }
            } else {
                let h = (rect.size.y - gap * (rows - 1) as f64) / rows as f64;
                (row as f64 * (h + gap), h)
            };
            self.panel_rects[id] = Rect {
                pos: rect.pos + dvec2(x, y),
                size: dvec2(w.max(0.0), h.max(0.0)),
            };
        }
        if cols == 2 {
            self.handles[0] = Rect {
                pos: rect.pos + dvec2((rect.size.x - gap) * self.state.split_x, 0.0),
                size: dvec2(gap, rect.size.y),
            };
        }
        if rows == 2 {
            self.handles[1] = Rect {
                pos: rect.pos + dvec2(0.0, (rect.size.y - gap) * self.state.split_y),
                size: dvec2(rect.size.x, gap),
            };
        }
        for (i, path) in PANELS.iter().enumerate() {
            let panel = self.view.widget(cx, path);
            panel.set_visible(cx, visible.contains(&i));
            if let Some(mut v) = panel.borrow_mut::<View>() {
                let r = self.panel_rects[i];
                v.walk = Walk {
                    abs_pos: Some(r.pos),
                    width: Size::Fixed(r.size.x),
                    height: Size::Fixed(r.size.y),
                    ..Walk::default()
                };
            }
            panel.button(cx, ids!(head.maximize)).set_text(
                cx,
                if self.state.maximized == Some(i) {
                    "⤡"
                } else {
                    "⤢"
                },
            );
        }
        self.view
            .widget(cx, ids!(empty))
            .set_visible(cx, visible.is_empty());
    }
}
impl Widget for FlexPanels {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        let actions = cx.capture_actions(|cx| self.view.handle_event(cx, event, scope));
        for (i, path) in PANELS.iter().enumerate() {
            let panel = self.view.widget(cx, path);
            if panel.button(cx, ids!(head.close)).clicked(&actions) {
                self.state.toggle(i);
                self.changed(cx);
            }
            if panel.button(cx, ids!(head.maximize)).clicked(&actions) {
                self.state.maximized = if self.state.maximized == Some(i) {
                    None
                } else {
                    Some(i)
                };
                self.changed(cx);
            }
        }
        cx.extend_actions(actions);
        match event {
            Event::MouseDown(e) if self.rect.contains(e.abs) => {
                for (i, r) in self.handles.iter().enumerate() {
                    if r.contains(e.abs) && r.size.x > 0.0 {
                        self.resize = Some(i == 0);
                        return;
                    }
                }
                for (i, r) in self.panel_rects.iter().enumerate() {
                    if r.contains(e.abs)
                        && e.abs.y < r.pos.y + 44.0
                        && e.abs.x < r.pos.x + r.size.x - 88.0
                    {
                        self.drag = Some((i, e.abs));
                        break;
                    }
                }
            }
            Event::MouseMove(e) => {
                if let Some(horizontal) = self.resize {
                    if horizontal {
                        self.state.split_x =
                            ((e.abs.x - self.rect.pos.x) / self.rect.size.x).clamp(0.2, 0.8);
                    } else {
                        self.state.split_y =
                            ((e.abs.y - self.rect.pos.y) / self.rect.size.y).clamp(0.2, 0.8);
                    }
                    self.view.redraw(cx);
                } else if let Some((source, start)) = self.drag {
                    if (e.abs - start).length() > 8.0 {
                        self.drop_target = self
                            .panel_rects
                            .iter()
                            .enumerate()
                            .find(|(i, r)| *i != source && r.contains(e.abs))
                            .map(|(i, _)| i);
                        self.view.redraw(cx);
                    }
                } else if self.handles[0].contains(e.abs) {
                    cx.set_cursor(MouseCursor::ColResize);
                } else if self.handles[1].contains(e.abs) {
                    cx.set_cursor(MouseCursor::RowResize);
                }
            }
            Event::MouseUp(_) => {
                if let Some((source, _)) = self.drag.take() {
                    if let Some(target) = self.drop_target.take() {
                        self.state.move_before(source, target);
                    }
                    self.changed(cx);
                }
                if self.resize.take().is_some() {
                    self.changed(cx);
                }
            }
            Event::KeyDown(e) if e.key_code == KeyCode::Escape => {
                self.drag = None;
                self.drop_target = None;
                self.resize = None;
                self.state.maximized = None;
                self.changed(cx);
            }
            _ => {}
        }
    }
    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        let light = crate::ui::frame::light_mode();
        if self.themed_light != light {
            self.themed_light = light;
            for path in PANELS {
                let mut panel = self.view.widget(cx, path);
                script_apply_eval!(cx, panel, {draw_bg +: {light: #(light)}});
                crate::ui::frame::apply_light(
                    cx,
                    &panel,
                    &[
                        (ids!(head.maximize), crate::ui::frame::Themed::Both),
                        (ids!(head.close), crate::ui::frame::Themed::Both),
                    ],
                    light,
                );
            }
        }
        if !self.loaded {
            self.state = PanelLayout::load(&self.persistence_key);
            for i in self.panel_count.min(4)..4 {
                self.state.visible[i] = false;
            }
            self.loaded = true;
        }
        let rect = cx.peek_walk_turtle(walk);
        self.place(cx, rect);
        let result = self.view.draw_walk(cx, scope, walk);
        if let Some(id) = self.drop_target {
            self.preview.color = vec4(0.04, 0.48, 1.0, 0.18);
            self.preview.draw_abs(cx, self.panel_rects[id]);
        }
        result
    }
}
impl FlexPanelsRef {
    pub fn snapshot(&self) -> Option<PanelLayout> {
        self.borrow().map(|v| v.state.clone())
    }
    pub fn toggle_panel(&self, cx: &mut Cx, id: usize) {
        if let Some(mut v) = self.borrow_mut() {
            v.state.toggle(id);
            v.changed(cx);
        }
    }
    pub fn reset(&self, cx: &mut Cx) {
        if let Some(mut v) = self.borrow_mut() {
            v.state = PanelLayout::default();
            for i in v.panel_count.min(4)..4 {
                v.state.visible[i] = false;
            }
            v.changed(cx);
        }
    }
    pub fn cycle_layout(&self, cx: &mut Cx) {
        if let Some(mut v) = self.borrow_mut() {
            v.state.columns = match v.state.columns {
                2 => 1,
                1 => 4,
                _ => 2,
            };
            v.state.maximized = None;
            v.changed(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reorder_close_restore_preserves_all_identities() {
        let mut s = PanelLayout::default();
        s.move_before(3, 0);
        s.toggle(1);
        let mut restored: PanelLayout =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(restored.order, [3, 0, 1, 2]);
        assert!(!restored.visible[1]);
        restored.toggle(1);
        assert!(restored.visible[1]);
        assert!(restored.validate());
    }
    #[test]
    fn rejects_missing_duplicate_and_invalid_panels() {
        let mut s = PanelLayout::default();
        s.order = vec![0, 0, 2, 3];
        assert!(!s.validate());
        s = PanelLayout::default();
        s.maximized = Some(8);
        assert!(!s.validate());
        s = PanelLayout::default();
        s.split_x = f64::NAN;
        assert!(!s.validate());
    }
}
