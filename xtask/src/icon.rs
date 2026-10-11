//! 原创 MailFabrica 图标：蓝色圆角底、白色信封与青色交织线。源素材全由代码绘制。

use std::path::Path as FilePath;

use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Paint, Path, PathBuilder, Pixmap, Point, SpreadMode, Stroke, Transform,
};

/// 绘制指定尺寸的抗锯齿 PNG；输入尺寸必须为 16–1024，分配或写入失败时返回错误。
pub(crate) fn render(size: u32, output: &FilePath) -> Result<(), String> {
    if !(16..=1024).contains(&size) {
        return Err("icon size must be between 16 and 1024".into());
    }
    let mut image = Pixmap::new(size, size).ok_or("cannot allocate icon pixels")?;
    let transform = Transform::from_scale(size as f32 / 1024.0, size as f32 / 1024.0);
    let gradient = LinearGradient::new(
        Point::from_xy(100.0, 100.0),
        Point::from_xy(924.0, 924.0),
        vec![GradientStop::new(0.0, Color::from_rgba8(55, 133, 250, 255)), GradientStop::new(1.0, Color::from_rgba8(19, 62, 175, 255))],
        SpreadMode::Pad,
        Transform::identity(),
    )
    .ok_or("cannot create icon gradient")?;
    let background = Paint { shader: gradient, ..Paint::default() };
    let mut shadow = Paint::default();
    shadow.set_color_rgba8(13, 36, 88, 24);
    image.fill_path(&rounded_rect(100.0, 116.0, 824.0, 824.0, 184.0)?, &shadow, FillRule::Winding, transform, None);
    image.fill_path(&rounded_rect(100.0, 100.0, 824.0, 824.0, 184.0)?, &background, FillRule::Winding, transform, None);
    let mut white = Paint::default();
    white.set_color_rgba8(255, 255, 255, 255);
    let mut cyan = Paint::default();
    cyan.set_color_rgba8(107, 231, 226, 255);
    let stroke = Stroke { width: 44.0, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
    image.stroke_path(&rounded_rect(246.0, 334.0, 532.0, 356.0, 44.0)?, &white, &stroke, transform, None);
    image.stroke_path(&line(&[(270.0, 660.0), (433.0, 516.0)])?, &white, &stroke, transform, None);
    image.stroke_path(&line(&[(591.0, 516.0), (754.0, 660.0)])?, &cyan, &stroke, transform, None);
    // 上层信封折线遮住交叉点的一小段底线，形成清楚的“经纬交织”，而不是两个粘连的线条。
    let flap = line(&[(264.0, 368.0), (512.0, 562.0), (760.0, 368.0)])?;
    let gap = Stroke { width: 70.0, ..stroke.clone() };
    image.stroke_path(&flap, &background, &gap, transform, None);
    image.stroke_path(&flap, &white, &stroke, transform, None);
    image.save_png(output).map_err(|e| format!("{}: {e}", output.display()))
}

/// 使用三次贝塞尔曲线逼近圆角，所有尺寸基于 1024 的同一坐标系。
fn rounded_rect(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Result<Path, String> {
    // 四分之一圆的标准贝塞尔控制距离。
    let k = radius * 0.552_284_8;
    let (right, bottom) = (x + width, y + height);
    let mut p = PathBuilder::new();
    p.move_to(x + radius, y);
    p.line_to(right - radius, y);
    p.cubic_to(right - radius + k, y, right, y + radius - k, right, y + radius);
    p.line_to(right, bottom - radius);
    p.cubic_to(right, bottom - radius + k, right - radius + k, bottom, right - radius, bottom);
    p.line_to(x + radius, bottom);
    p.cubic_to(x + radius - k, bottom, x, bottom - radius + k, x, bottom - radius);
    p.line_to(x, y + radius);
    p.cubic_to(x, y + radius - k, x + radius - k, y, x + radius, y);
    p.close();
    p.finish().ok_or_else(|| "cannot create rounded icon path".into())
}

/// 创建折线路径；不接受空点集。
fn line(points: &[(f32, f32)]) -> Result<Path, String> {
    let Some(&(x, y)) = points.first() else { return Err("icon line is empty".into()) };
    let mut p = PathBuilder::new();
    p.move_to(x, y);
    for &(x, y) in points.iter().skip(1) {
        p.line_to(x, y);
    }
    p.finish().ok_or_else(|| "cannot create icon line".into())
}
