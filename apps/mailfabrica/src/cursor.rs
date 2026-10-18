//! macOS 光标归属：只移除父视图的光标区域，按当前文档命中结果校正原生光标。

use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2_app_kit::{NSCursor, NSWindow};
use wry::{WebView, WebViewExtMacOS};

/// 每个正文实例独立保存悬停结果；坐标变化、换信、隐藏或失焦后不复用旧结果。
#[derive(Default)]
pub(crate) struct CursorOwner {
    window: Option<Retained<NSWindow>>,
    suppressed: bool,
    sample: Option<([f64; 2], String, Instant)>,
    queried: Option<Instant>,
    sequence: u64,
    pending: bool,
}

impl CursorOwner {
    /// 读取真实指针的 CSS 视口坐标；窗口的光标机制保持开启，WebKit 仍能处理自己的跟踪区域。
    pub(crate) fn update(&mut self, view: Option<&WebView>) -> Option<[f64; 2]> {
        let point = view.and_then(|view| {
            let native = view.webview();
            let window = native.window()?;
            let point = native.convertPoint_fromView(window.mouseLocationOutsideOfEventStream(), None);
            let bounds = native.bounds();
            let y = if native.isFlipped() { point.y - bounds.origin.y } else { bounds.size.height - (point.y - bounds.origin.y) };
            if !window.isKeyWindow()
                || point.x < bounds.origin.x
                || point.y < bounds.origin.y
                || point.x >= bounds.origin.x + bounds.size.width
                || point.y >= bounds.origin.y + bounds.size.height
            {
                return None;
            }
            // winit 的整窗箭头区域会覆盖子视图，只移除父视图自己的区域，不禁用整个窗口。
            if let Some(root) = window.contentView() {
                root.discardCursorRects();
            }
            self.window = Some(window);
            self.suppressed = true;
            Some([point.x - bounds.origin.x, y])
        });
        if point.is_none() {
            self.reset();
        }
        point
    }

    /// 最多每 50 毫秒查询一次；超时后允许重试，迟到结果通过序号丢弃。
    pub(crate) fn query(&mut self) -> Option<u64> {
        let elapsed = self.queried.map(|at| at.elapsed());
        if elapsed.is_some_and(|d| d < Duration::from_millis(if self.pending { 250 } else { 50 })) {
            return None;
        }
        self.sequence = self.sequence.wrapping_add(1);
        self.pending = true;
        self.queried = Some(Instant::now());
        Some(self.sequence)
    }

    /// 保存当前查询的结果；不会让较早的异步回调覆盖最新指针位置。
    pub(crate) fn complete(&mut self, sequence: u64, point: [f64; 2], result: &str) {
        if sequence != self.sequence {
            return;
        }
        self.pending = false;
        self.sample = serde_json::from_str::<String>(result).ok().map(|cursor| (point, cursor, Instant::now()));
    }

    /// 只应用当前坐标的近期结果；不等待图片或导航完成，重复应用可抵消父视图重建的箭头。
    pub(crate) fn apply(&self, point: [f64; 2]) -> Option<&str> {
        let (at, cursor, checked) = self.sample.as_ref()?;
        if !same_point(*at, point) || checked.elapsed() > Duration::from_millis(250) {
            return None;
        }
        match cursor.as_str() {
            "pointer" => NSCursor::pointingHandCursor().set(),
            "text" => NSCursor::IBeamCursor().set(),
            "default" => NSCursor::arrowCursor().set(),
            _ => return None,
        }
        Some(cursor)
    }

    /// 离开、隐藏或换信时恢复父视图，清空待处理结果和查询节流。
    pub(crate) fn reset(&mut self) {
        if self.suppressed {
            if let Some(window) = &self.window
                && let Some(root) = window.contentView()
            {
                window.invalidateCursorRectsForView(&root);
            }
            self.suppressed = false;
        }
        self.sample = None;
        self.pending = false;
        self.queried = None;
        self.sequence = self.sequence.wrapping_add(1);
    }
}

impl Drop for CursorOwner {
    /// 退出时恢复父视图的光标区域。
    fn drop(&mut self) {
        self.reset();
    }
}

/// 原生坐标以逻辑像素计算；微小浮点误差不能让已经命中的结果失效。
fn same_point(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() < 0.01
}

/// 仅执行宿主的固定命中脚本；不安装网页事件处理器，不开放邮件脚本或 IPC。
pub(crate) fn hit_test(point: [f64; 2]) -> String {
    let [x, y] = point;
    format!(
        r#"(() => {{
 const x={x},y={y},e=document.elementFromPoint(x,y);if(!e)return 'default';
 if(e.closest('a[href]'))return 'pointer';
 const c=getComputedStyle(e).cursor;
 if(['pointer','text','default'].includes(c))return c;
 if(c!=='auto')return 'native';
 const r=document.caretRangeFromPoint(x,y);
 if(r&&r.startContainer.nodeType===Node.TEXT_NODE){{
  const n=r.startContainer,o=Math.min(r.startOffset,Math.max(0,n.length-1));
  if(n.length){{r.setStart(n,o);r.setEnd(n,o+1);const b=r.getBoundingClientRect();
   if(x>=b.left&&x<=b.right&&y>=b.top&&y<=b.bottom)return 'text';}}
 }}
 return 'default';
}})()"#
    )
}

/// 读取本应用实际光标；不能用 DOM 样式或自动化操作指示器代替原生悬停验收。
pub(crate) fn current_name() -> &'static str {
    let current = NSCursor::currentCursor();
    if std::ptr::eq(&*current, &*NSCursor::pointingHandCursor()) {
        "hand"
    } else if std::ptr::eq(&*current, &*NSCursor::IBeamCursor()) {
        "text"
    } else if std::ptr::eq(&*current, &*NSCursor::arrowCursor()) {
        "arrow"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 换信、隐藏和较晚返回的悬停结果都不能复活上一封邮件的手形。
    #[test]
    fn rejects_old_cursor_results() {
        let mut owner = CursorOwner::default();
        let first = owner.query().expect("query");
        owner.complete(first, [10.0, 20.0], "\"pointer\"");
        assert!(owner.sample.is_some());
        owner.reset();
        owner.complete(first, [10.0, 20.0], "\"pointer\"");
        assert!(owner.sample.is_none());
        let second = owner.query().expect("new query");
        owner.complete(first, [10.0, 20.0], "\"pointer\"");
        assert!(owner.pending);
        owner.complete(second, [30.0, 40.0], "\"default\"");
        let (point, cursor, _) = owner.sample.as_ref().expect("sample");
        assert_eq!(cursor, "default");
        assert!(!same_point(*point, [10.0, 20.0]));
        assert!(owner.apply([10.0, 20.0]).is_none());
        assert!(owner.query().is_none());
        owner.sample = Some(([30.0, 40.0], "pointer".into(), Instant::now() - Duration::from_millis(251)));
        assert!(owner.apply([30.0, 40.0]).is_none());
    }
}
