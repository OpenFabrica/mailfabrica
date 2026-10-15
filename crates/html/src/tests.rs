use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::*;

/// 不联网、什么也不叫醒的选项。
fn offline() -> Options {
    Options { remote_images: false, inline: vec![], wake: Arc::new(|| {}) }
}

/// 邮件布局表格 border=0 必须没有黑框；嵌套的单元格也一样。
#[test]
fn borderless_email_tables_have_no_black_frames() {
    let html = r#"<style>table {border-collapse:collapse}</style><body style="margin:0;background:white"><table border="0" cellpadding="0" cellspacing="0" width="300"><tr><td>
        <table border="0" cellpadding="0" cellspacing="0" width="300"><tr><td style="height:80px;background:white"></td></tr></table>
        </td></tr></table></body>"#;
    let start = Instant::now();
    let mut v = HtmlView::new(html, offline());
    v.set_size(400.0, 1.0);
    pump(&mut v, |v, _| v.layout().is_some());
    v.request_tile(0);
    let ev = pump(&mut v, |_, ev| ev.iter().any(|e| matches!(e, Event::Tile(_))));
    let tile = ev.into_iter().find_map(|e| if let Event::Tile(t) = e { Some(t) } else { None }).expect("tile");
    let dark = tile.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] > 0 && p[0] < 100 && p[1] < 100 && p[2] < 100).count();
    eprintln!("borderless table: {dark} dark pixels, first tile {:?}", start.elapsed());
    assert_eq!(dark, 0, "border=0 layout tables must not paint dark borders");
    assert!((v.layout().expect("layout").height - 80.0).abs() < 1.0, "cellspacing=0 must not add gaps: {:?}", v.layout());
}

/// 表格单元格的显式单边装饰只画在指定边上，不能蔓延成整张表的边框。
#[test]
fn table_cell_top_border_does_not_frame_the_table() {
    let html = r#"<style>table{border-collapse:collapse}</style><body style="margin:0;background:white">
        <table border="0" cellspacing="0" cellpadding="0" width="300"><tr><td style="height:80px;border-top:10px solid #ff8000"></td></tr></table></body>"#;
    let mut v = HtmlView::new(html, offline());
    v.set_size(400.0, 1.0);
    pump(&mut v, |v, _| v.layout().is_some());
    v.request_tile(0);
    let events = pump(&mut v, |_, e| e.iter().any(|e| matches!(e, Event::Tile(_))));
    let tile = events.into_iter().find_map(|e| if let Event::Tile(t) = e { Some(t) } else { None }).expect("tile");
    let pixel = |x: u32, y: u32| tile.rgba.get(((y * tile.width + x) * 4) as usize..((y * tile.width + x) * 4 + 3) as usize);
    assert_eq!(pixel(150, 5), Some(&[255, 128, 0][..]), "authored orange top border");
    assert_eq!(pixel(1, 40), Some(&[255, 255, 255][..]), "no invented left border");
    assert_eq!(pixel(298, 40), Some(&[255, 255, 255][..]), "no invented right border");
}

/// cellspacing/cellpadding 非零值仍生效，作者 CSS 可以覆盖旧式属性。
#[test]
fn legacy_table_spacing_and_padding_respect_author_css() {
    for (style, expected) in [("", 94.0), ("table{border-spacing:0}td{padding:2px}", 84.0)] {
        let html = format!(
            r#"<style>{style}</style><body style="margin:0"><table cellspacing="3" cellpadding="4"><tr><td><div style="height:80px;width:100px"></div></td></tr></table></body>"#
        );
        let mut v = HtmlView::new(&html, offline());
        v.set_size(400.0, 1.0);
        pump(&mut v, |v, _| v.layout().is_some());
        assert!((v.layout().expect("layout").height - expected).abs() < 1.0, "{:?}", v.layout());
    }
}

/// 纹理被界面淘汰后，再请求同一代同一图块仍能得到像素，避免长信滚回顶部空白。
#[test]
fn tile_can_be_requested_again_after_delivery() {
    let mut v = HtmlView::new("<body style='margin:0'><p>Hello</p><div style='height:2000px'></div></body>", offline());
    v.set_size(400.0, 1.0);
    pump(&mut v, |v, _| v.layout().is_some());
    for _ in 0..2 {
        v.request_tile(0);
        let events = pump(&mut v, |_, e| e.iter().any(|e| matches!(e, Event::Tile(t) if t.index == 0)));
        assert!(events.iter().any(|e| matches!(e, Event::Tile(t) if t.index == 0)), "tile was permanently marked as requested");
    }
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

/// 邮件里的图片地址不能指向本机或局域网（防止替发件人请求内网服务）。
#[test]
fn only_public_addresses_are_fetched() {
    let ok = |s: &str| net::is_public(s.parse().unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)));
    for bad in [
        "127.0.0.1",
        "10.1.2.3",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "0.0.0.0",
        "224.0.0.1",
        "255.255.255.255",
        "::1",
        "::",
        "fd00::1",
        "fe80::1",
        "::ffff:127.0.0.1",
        "::ffff:192.168.0.1",
    ] {
        assert!(!ok(bad), "{bad} must be blocked");
    }
    for good in ["8.8.8.8", "142.250.72.14", "198.18.30.39", "2606:4700:4700::1111", "::ffff:8.8.8.8"] {
        assert!(ok(good), "{good} must be allowed");
    }
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

/// 打开远程图片时，指向本机的图片地址收不到任何连接。
#[test]
fn local_image_urls_are_not_requested() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let opts = Options { remote_images: true, inline: vec![], wake: Arc::new(|| {}) };
    let html = format!(r#"<body><img src="http://127.0.0.1:{port}/a.png"><img src="http://localhost:{port}/b.png"></body>"#);
    let mut v = HtmlView::new(&html, opts);
    v.set_size(300.0, 1.0);
    pump(&mut v, |v, _| v.layout().is_some());
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(1500) {
        v.poll();
        assert!(listener.accept().is_err(), "a local address was requested");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 排好版后画第一块图。
fn first_tile(html: &str, width: f32) -> Tile {
    let mut v = HtmlView::new(html, offline());
    v.set_size(width, 1.0);
    pump(&mut v, |v, _| v.layout().is_some());
    v.request_tile(0);
    let ev = pump(&mut v, |_, e| e.iter().any(|e| matches!(e, Event::Tile(_))));
    ev.into_iter().find_map(|e| if let Event::Tile(t) = e { Some(t) } else { None }).expect("tile")
}

/// 图块里 (x, y) 处的 RGB。
fn rgb(t: &Tile, x: usize, y: usize) -> Vec<u8> {
    let i = (y * t.width as usize + x) * 4;
    t.rgba.get(i..i + 3).map(<[u8]>::to_vec).unwrap_or_default()
}

/// `<body>` 上的宽度限制不生效（Gmail 也不认）：否则整封信被挤到中间偏右，宽表格从右边溢出。
#[test]
fn body_width_styles_are_ignored() {
    let html = r#"<body style="max-width:200px;margin:0 auto"><div style="width:600px;height:20px;background:#ff0000"></div></body>"#;
    let t = first_tile(html, 800.0);
    assert_eq!(rgb(&t, 5, 10), vec![255, 0, 0], "content must start at the left edge");
}

/// `width="100%"` 的超宽图片（`late-` 开头的 cid 图片，在第一次排版之后才到，和远程图片一样）不能把
/// `max-width` 的表格撑宽：图片要限制在容器宽度内，图片到达后外层表格的排版缓存也要清掉重算。
#[test]
fn wide_percentage_image_keeps_table_max_width() {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(3000, 50, image::Rgba([0, 0, 255, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("png");
    let options = Options { remote_images: false, inline: vec![("late-wide@x".into(), png)], wake: Arc::new(|| {}) };
    // 结构仿 Meta 验证码邮件：外层 max-width 表格；头图在第一行，文字在第二行里嵌套的 width=100% 表格中
    let html = r#"<body style="margin:0"><table cellspacing="0" cellpadding="0" align="center" style="border-collapse:collapse;max-width:300px;margin:0 auto"><tr><td>
        <table width="100%" cellspacing="0" cellpadding="0" style="border-collapse:collapse">
        <tr><td><table width="100%" height="50" cellspacing="0" cellpadding="0" style="max-width:1400px;margin:0 auto"><tr>
          <td style="padding:0;overflow:hidden"><img width="100%" height="50" src="cid:late-wide@x" style="display:block;width:100%;object-fit:cover"></td>
        </tr></table></td></tr>
        <tr><td><table width="100%" cellspacing="0" cellpadding="0"><tr><td>
          <span style="display:block;font-size:15px;color:#000">A long paragraph of text that is much wider than three hundred pixels on one line.</span>
        </td></tr></table></td></tr>
        </table></td></tr></table></body>"#;
    let mut v = HtmlView::new(html, options);
    v.set_size(800.0, 1.0);
    // 图片到达后会重新排版：一直取最新一代的第一块图，直到图片画出来
    let start = Instant::now();
    let mut tile = None;
    while start.elapsed() < Duration::from_secs(20) {
        v.request_tile(0);
        for e in v.poll() {
            if let Event::Tile(t) = e {
                tile = Some(t);
            }
        }
        if tile.as_ref().is_some_and(|t| t.rgba.as_chunks::<4>().0.iter().any(|p| p[..3] == [0, 0, 255])) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let t = tile.expect("tile");
    // 表格 300 宽、居中：左右各 250
    assert_eq!(rgb(&t, 260, 25), vec![0, 0, 255], "the image must be shown");
    assert_ne!(rgb(&t, 600, 25), vec![0, 0, 255], "the image must not overflow the table");
    let dark_right = (50..t.height as usize)
        .filter_map(|y| (0..800).rev().find(|&x| t.rgba[(y * 800 + x) * 4 + 3] > 200 && rgb(&t, x, y).iter().all(|c| *c < 90)))
        .max();
    assert!(dark_right.is_some_and(|x| x < 555), "text must wrap inside the 300px table: rightmost dark pixel {dark_right:?}");
}

/// `width="100%"` 的超宽图片（一开始就有）要限制在容器宽度内，不能把 `max-width` 的表格按原图宽度撑开。
#[test]
fn wide_percentage_image_is_capped_to_its_container() {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(3000, 50, image::Rgba([0, 0, 255, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("png");
    let options = Options { remote_images: false, inline: vec![("wide@x".into(), png)], wake: Arc::new(|| {}) };
    let html = r#"<body style="margin:0"><table cellspacing="0" cellpadding="0" align="center" style="max-width:300px;margin:0 auto"><tr><td>
        <table width="100%" cellspacing="0" cellpadding="0"><tr><td><img width="100%" height="50" src="cid:wide@x" style="display:block;width:100%"></td></tr></table>
        </td></tr><tr><td><div style="height:20px;background:#ff0000"></div></td></tr></table></body>"#;
    let mut v = HtmlView::new(html, options);
    v.set_size(800.0, 1.0);
    pump(&mut v, |v, _| v.layout().is_some());
    v.request_tile(0);
    let ev = pump(&mut v, |_, e| e.iter().any(|e| matches!(e, Event::Tile(_))));
    let t = ev.into_iter().find_map(|e| if let Event::Tile(t) = e { Some(t) } else { None }).expect("tile");
    assert_eq!(rgb(&t, 400, 25), vec![0, 0, 255]);
    assert_eq!(rgb(&t, 400, 60), vec![255, 0, 0]);
    assert_ne!(rgb(&t, 600, 60), vec![255, 0, 0], "the table must stay 300px wide");
}

/// 从空白处（左边距、文字下面）开始或结束拖动也能选中文字：吸附到最近的文字，和浏览器一样。
#[test]
fn selection_snaps_to_nearest_text() {
    let html = r#"<body style="margin:0"><p style="margin:0 0 0 200px;font-size:20px">Hello world</p><p style="margin:40px 0 0 200px;font-size:20px">Second line</p></body>"#;
    let mut v = HtmlView::new(html, offline());
    v.set_size(800.0, 1.0);
    pump(&mut v, |v, _| v.layout().is_some());
    // 从左边空白处拖到第二段右边很远的地方
    v.pointer(Pointer::Down { x: 10.0, y: 10.0, shift: false });
    v.pointer(Pointer::Drag { x: 790.0, y: 75.0 });
    v.pointer(Pointer::Up { x: 790.0, y: 75.0 });
    v.copy();
    let ev = pump(&mut v, |_, e| e.iter().any(|e| matches!(e, Event::Copied(_))));
    let copied = ev.into_iter().find_map(|e| if let Event::Copied(s) = e { Some(s) } else { None }).unwrap_or_default();
    assert!(copied.contains("Hello world") && copied.contains("Second line"), "{copied:?}");
    // 选中的部分画了背景色
    v.request_tile(0);
    let ev = pump(&mut v, |_, e| e.iter().any(|e| matches!(e, Event::Tile(_))));
    let tile = ev.into_iter().find_map(|e| if let Event::Tile(t) = e { Some(t) } else { None }).expect("tile");
    let highlighted = tile.rgba.as_chunks::<4>().0.iter().filter(|p| p[0] == 180 && p[1] == 213 && p[2] == 255).count();
    assert!(highlighted > 500, "{highlighted} highlighted pixels");
    // 在空白处单击：选区收起，没有高亮
    v.pointer(Pointer::Down { x: 10.0, y: 200.0, shift: false });
    v.pointer(Pointer::Up { x: 10.0, y: 200.0 });
    pump(&mut v, |v, _| !v.has_selection());
    assert!(!v.has_selection());
}
