use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::*;

/// 不联网、什么也不叫醒的选项。
fn offline() -> Options {
    Options { remote_images: false, inline: vec![], wake: Arc::new(|| {}) }
}

/// 一直 poll 到 `done` 为真或超时，返回期间收到的事件。
fn pump(v: &mut HtmlView, mut done: impl FnMut(&HtmlView, &[Event]) -> bool) -> Vec<Event> {
    let start = Instant::now();
    let mut all = Vec::new();
    while start.elapsed() < Duration::from_secs(20) {
        all.extend(v.poll());
        if done(v, &all) {
            return all;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    all
}

#[test]
fn strips_external_styles() {
    let s = strip_external_styles(r#"<head><LINK rel="stylesheet" href="//x/a.css"/><linkx>t</linkx><style>@IMPORT url(a.css); p{}</style></head>"#);
    assert_eq!(s, r#"<head><linkx>t</linkx><style>@x-ignored-import url(a.css); p{}</style></head>"#);
    // 结尾不完整的 `<link` 不是标签，原样保留；多字节文字旁边也不会切坏
    assert_eq!(strip_external_styles("中文<link"), "中文<link");
    assert_eq!(strip_external_styles("中文<link href=x>文"), "中文文");
}

#[test]
fn data_and_cid_urls() {
    assert_eq!(net::decode_data_url("data:image/png;base64,aGk"), Some(b"hi".to_vec()));
    assert_eq!(net::decode_data_url("data:text/plain,a%20b"), Some(b"a b".to_vec()));
    assert_eq!(net::cid_key("%3CLogo@X%3E"), "logo@x");
}

/// 排版、画块、点链接、选文字复制，整条路走一遍。
#[test]
fn layout_paint_click_and_copy() {
    let html = r#"<!DOCTYPE html><html><body style="margin:0">
        <div style="height:40px">Hello <b>world</b></div>
        <a href="https://example.com/x" style="display:block;height:30px;background:#0b57d0;color:#fff">Check activity</a>
        <div style="height:1200px"></div></body></html>"#;
    let mut v = HtmlView::new(html, offline());
    v.set_size(400.0, 2.0);
    pump(&mut v, |v, _| v.layout().is_some());
    let l = v.layout().expect("layout");
    assert_eq!((l.width, l.scale), (400.0, 2.0));
    assert!(l.height >= 1270.0, "{l:?}");
    assert_eq!(l.tiles(), 3);
    v.request_tile(0);
    let ev = pump(&mut v, |_, e| e.iter().any(|e| matches!(e, Event::Tile(_))));
    let Some(Event::Tile(t)) = ev.into_iter().find(|e| matches!(e, Event::Tile(_))) else { panic!("no tile") };
    assert_eq!((t.width, t.height, t.rgba.len()), (800, 1024, 800 * 1024 * 4));
    // 按钮区域是蓝色（第 55 CSS 像素那一行）
    let px = |x: usize, y: usize| t.rgba.get((y * 800 + x) * 4..(y * 800 + x) * 4 + 3).map(<[u8]>::to_vec);
    assert_eq!(px(790, 110), Some(vec![0x0b, 0x57, 0xd0]));
    // 悬停和点击链接
    v.pointer(Pointer::Hover { x: 380.0, y: 55.0 });
    pump(&mut v, |v, _| v.hover_link().is_some());
    assert_eq!(v.hover_link(), Some("https://example.com/x"));
    v.pointer(Pointer::Down { x: 380.0, y: 55.0, shift: false });
    v.pointer(Pointer::Up { x: 380.0, y: 55.0 });
    let ev = pump(&mut v, |_, e| e.iter().any(|e| matches!(e, Event::OpenLink(_))));
    assert!(ev.iter().any(|e| matches!(e, Event::OpenLink(u) if u == "https://example.com/x")));
    // 从 Hello 前面拖到 world 后面，复制出来
    v.pointer(Pointer::Down { x: 1.0, y: 10.0, shift: false });
    v.pointer(Pointer::Drag { x: 300.0, y: 10.0 });
    v.pointer(Pointer::Up { x: 300.0, y: 10.0 });
    v.copy();
    let ev = pump(&mut v, |_, e| e.iter().any(|e| matches!(e, Event::Copied(_))));
    let copied = ev.into_iter().find_map(|e| if let Event::Copied(s) = e { Some(s) } else { None }).unwrap_or_default();
    assert!(copied.contains("Hello") && copied.contains("world"), "{copied:?}");
    assert!(v.has_selection());
    assert!(v.failed().is_none());
}

/// 内嵌图片（cid:）加载后重新排版，高度变成图片的高度。
#[test]
fn cid_images_load() {
    // 1×1 的 PNG
    let png = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
    )
    .unwrap_or_default();
    let (tx, rx) = mpsc::channel::<()>();
    let tx = std::sync::Mutex::new(tx);
    let opts = Options {
        remote_images: false,
        inline: vec![("<Logo@X>".into(), png)],
        wake: Arc::new(move || {
            let _ = tx.lock().map(|t| t.send(()));
        }),
    };
    let mut v = HtmlView::new(r#"<body style="margin:0"><img src="cid:logo@x" style="display:block;height:auto;width:300px"></body>"#, opts);
    v.set_size(400.0, 1.0);
    pump(&mut v, |v, _| v.layout().is_some_and(|l| l.height >= 299.0));
    assert!(rx.try_recv().is_ok(), "wake was called");
    assert!(v.layout().is_some_and(|l| l.height >= 299.0), "{:?}", v.layout());
}

/// 丢掉句柄后渲染线程会退出（文档里的 Shell 也拿着发送端，不能只靠通道关闭）。
#[test]
fn renderer_thread_exits_on_drop() {
    let (tx, rx) = mpsc::channel::<()>();
    struct Guard(mpsc::Sender<()>);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    // wake 闭包随 Options 进入渲染线程，线程退出时丢掉它，Guard 就发信号
    let guard = Arc::new(Guard(tx));
    let opts = Options { remote_images: false, inline: vec![], wake: Arc::new(move || drop(guard.clone())) };
    let mut v = HtmlView::new("<p>hi</p>", opts);
    v.set_size(300.0, 1.0);
    pump(&mut v, |v, _| v.layout().is_some());
    drop(v);
    assert!(rx.recv_timeout(Duration::from_secs(10)).is_ok(), "renderer thread did not exit");
}

/// 能点到链接的区域要和画出来的按钮对齐：`display:inline-block` 加内边距的按钮（邮件里最常见的写法），
/// 按钮蓝色区域顶部能点到，按钮下面点不到。
#[test]
fn link_hit_area_matches_painted_button() {
    let html = r#"<body style="margin:0"><div style="padding-top:32px;text-align:center;font-size:14px">
        <a href="https://example.com/b" style="line-height:16px;color:#fff;text-decoration:none;font-size:14px;display:inline-block;
        padding:12px 24px;background-color:#0b57d0;border-radius:9999px;min-width:64px">Check activity</a></div>
        <div style="height:100px"></div></body>"#;
    let mut v = HtmlView::new(html, offline());
    v.set_size(400.0, 1.0);
    pump(&mut v, |v, _| v.layout().is_some());
    v.request_tile(0);
    let ev = pump(&mut v, |_, e| e.iter().any(|e| matches!(e, Event::Tile(_))));
    let Some(Event::Tile(t)) = ev.into_iter().find(|e| matches!(e, Event::Tile(_))) else { panic!("no tile") };
    // 中间那一列里蓝色像素的上下边
    let blue = |y: u32| t.rgba.get(((y * t.width + 200) * 4) as usize..((y * t.width + 200) * 4 + 3) as usize) == Some(&[0x0b, 0x57, 0xd0][..]);
    let rows: Vec<u32> = (0..t.height).filter(|y| blue(*y)).collect();
    let (top, bottom) = (rows.first().copied().unwrap_or(0) as f32, rows.last().copied().unwrap_or(0) as f32);
    assert!(bottom - top > 30.0, "button painted at {top}..{bottom}");
    let hover = |v: &mut HtmlView, y: f32| {
        v.pointer(Pointer::Hover { x: 200.0, y });
        // 先挪到空白处再挪回来，保证每次都有回复
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(300) {
            v.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        v.hover_link().map(str::to_string)
    };
    assert_eq!(hover(&mut v, top + 3.0).as_deref(), Some("https://example.com/b"), "top of the button ({top}..{bottom})");
    assert_eq!(hover(&mut v, bottom - 3.0).as_deref(), Some("https://example.com/b"), "bottom of the button");
    assert_eq!(hover(&mut v, bottom + 6.0), None, "below the button");
    assert_eq!(hover(&mut v, top - 6.0), None, "above the button");
}
