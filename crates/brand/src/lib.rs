//! 原创 MailFabrica 图标：蓝色圆角底、白色信封与青色交织线。源素材全由代码绘制。

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Paint, Path, PathBuilder, Pixmap, Point, SpreadMode, Stroke, Transform,
};

/// 绘制指定尺寸的抗锯齿像素；输入尺寸必须为 16–1024，路径或分配失败时返回错误。
fn draw(size: u32) -> Result<Pixmap, String> {
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
    Ok(image)
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

/// 返回透明 PNG；源码不存放位图，所有平台使用相同的矢量路径。
pub fn png(size: u32) -> Result<Vec<u8>, String> {
    draw(size)?.encode_png().map_err(|e| e.to_string())
}

/// 返回未预乘的 RGBA 像素，供窗口系统直接设置图标。
pub fn rgba(size: u32) -> Result<Vec<u8>, String> {
    let image = draw(size)?;
    let mut bytes = Vec::with_capacity(image.data().len());
    for pixel in image.pixels() {
        let color = pixel.demultiply();
        bytes.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
    }
    Ok(bytes)
}

/// Windows ICO：六个尺寸的 PNG 图层；256 的宽高字段按 ICO 规范编码为 0。
pub fn ico() -> Result<Vec<u8>, String> {
    let sizes = [16_u32, 32, 48, 64, 128, 256];
    let images = sizes.into_iter().map(png).collect::<Result<Vec<_>, _>>()?;
    let mut bytes = vec![0, 0, 1, 0, 6, 0];
    let mut offset = 6_u32 + 16 * 6;
    for (size, image) in sizes.into_iter().zip(&images) {
        let dimension = if size == 256 { 0 } else { size as u8 };
        let length = u32::try_from(image.len()).map_err(|_| "icon image is too large")?;
        bytes.extend_from_slice(&[dimension, dimension, 0, 0]);
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&32_u16.to_le_bytes());
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(&offset.to_le_bytes());
        offset = offset.checked_add(length).ok_or("icon offsets overflow")?;
    }
    for image in images {
        bytes.extend(image);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 校验实际输出的大小、透明角和 ICO 中每个可解码的 PNG 图层。
    #[test]
    fn native_icon_pixels_and_layers_are_valid() {
        assert!(rgba(0).is_err());
        assert!(png(4096).is_err());
        let pixels = rgba(64).expect("rgba");
        assert_eq!(pixels.len(), 64 * 64 * 4);
        assert_eq!(&pixels[..4], &[0, 0, 0, 0]);
        let icon = ico().expect("ico");
        assert_eq!(&icon[..6], &[0, 0, 1, 0, 6, 0]);
        for i in 0..6 {
            let entry = 6 + i * 16;
            let number = |start| u32::from_le_bytes(icon[start..start + 4].try_into().expect("field")) as usize;
            let (length, offset) = (number(entry + 8), number(entry + 12));
            let image = Pixmap::decode_png(&icon[offset..offset + length]).expect("PNG layer");
            let size = if icon[entry] == 0 { 256 } else { u32::from(icon[entry]) };
            assert_eq!((image.width(), image.height()), (size, size));
        }
    }
}
