//! 开发者回归工具：从本地 HTML 渲染首块 PNG，打印排版和首屏耗时；默认不联网。

use std::sync::Arc;
use std::time::{Duration, Instant};

use mailfabrica_html::{Event, HtmlView, Options};

/// 读取本地输入，通过正式渲染线程生成首屏，失败或超时返回错误。
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let input = args.get(1).ok_or("usage: preview INPUT.html OUTPUT.png [--remote]")?;
    let output = args.get(2).ok_or("missing OUTPUT.png")?;
    let html = std::fs::read_to_string(input)?;
    for run in 0..3 {
        let start = Instant::now();
        let mut view = HtmlView::new(&html, Options { remote_images: args.iter().any(|a| a == "--remote"), inline: vec![], wake: Arc::new(|| {}) });
        view.set_size(880.0, 2.0);
        let mut layout_time = None;
        loop {
            let events = view.poll();
            if let Some(e) = view.failed() {
                return Err(e.into());
            }
            if let Some(layout) = view.layout() {
                if layout_time.is_none() {
                    layout_time = Some(start.elapsed());
                }
                view.request_tile(0);
                if let Some(tile) =
                    events.into_iter().find_map(|e| if let Event::Tile(t) = e { (t.generation == layout.generation).then_some(t) } else { None })
                {
                    if run == 0 {
                        image::save_buffer(output, &tile.rgba, tile.width, tile.height, image::ColorType::Rgba8)?;
                    }
                    println!("run={run} layout={:?} first_tile={:?} height={}", layout_time, start.elapsed(), layout.height);
                    break;
                }
            }
            if start.elapsed() > Duration::from_secs(20) {
                return Err("renderer timed out".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    Ok(())
}
