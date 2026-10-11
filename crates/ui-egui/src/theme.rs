//! 设计令牌（颜色、尺寸）和字体。
//!
//! 配色是我们自己的：沉稳的蓝色强调色、浅灰侧边栏、白色列表和阅读窗格。字体在运行时从操作系统读取：
//! 拉丁字母用系统界面字体，中文用系统里能找到的黑体类字体做后备，最后才是 egui 自带字体。

use std::sync::Arc;

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, Visuals};

/// 界面的全部颜色。浅色和深色各一套，所有绘制代码都从这里取色。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    /// 工具栏背景。
    pub toolbar: Color32,
    /// 侧边栏背景。
    pub sidebar: Color32,
    /// 邮件列表背景。
    pub list: Color32,
    /// 阅读窗格背景。
    pub reader: Color32,
    /// 状态栏背景。
    pub status: Color32,
    /// 日期分组头背景。
    pub group: Color32,
    pub separator: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub text_disabled: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    /// 强调色上的文字。
    pub on_accent: Color32,
    /// 选中行背景。
    pub selected: Color32,
    /// 选中行文字。
    pub selected_text: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub input_bg: Color32,
    pub input_border: Color32,
    pub menu_bg: Color32,
    pub shadow: Color32,
    pub danger: Color32,
    pub success: Color32,
    pub flag: Color32,
    /// 侧边栏未读角标背景。
    pub badge: Color32,
    /// 附件卡片背景。
    pub chip: Color32,
    /// 头像底色，按邮箱哈希取一个。
    pub avatars: [Color32; 8],
}

/// 头像底色（两套主题共用，饱和度适中，白字可读）。
const AVATARS: [Color32; 8] = [
    Color32::from_rgb(0x2D, 0x7F, 0xF9),
    Color32::from_rgb(0x12, 0x9C, 0x8A),
    Color32::from_rgb(0xE5, 0x7A, 0x2E),
    Color32::from_rgb(0x8B, 0x5C, 0xD6),
    Color32::from_rgb(0xD8, 0x4A, 0x6B),
    Color32::from_rgb(0x3B, 0x9B, 0x3F),
    Color32::from_rgb(0x4A, 0x6F, 0xA5),
    Color32::from_rgb(0xB5, 0x8B, 0x1E),
];

impl Tokens {
    /// 浅色主题。
    pub fn light() -> Tokens {
        Tokens {
            dark: false,
            toolbar: Color32::from_rgb(0xF7, 0xF8, 0xFA),
            sidebar: Color32::from_rgb(0xEF, 0xF1, 0xF4),
            list: Color32::WHITE,
            reader: Color32::WHITE,
            status: Color32::from_rgb(0xF7, 0xF8, 0xFA),
            group: Color32::from_rgb(0xF5, 0xF6, 0xF8),
            separator: Color32::from_rgb(0xE1, 0xE4, 0xE8),
            text: Color32::from_rgb(0x1F, 0x23, 0x29),
            text_dim: Color32::from_rgb(0x5A, 0x60, 0x6B),
            text_faint: Color32::from_rgb(0x8A, 0x90, 0x99),
            text_disabled: Color32::from_rgb(0xB8, 0xBC, 0xC2),
            accent: Color32::from_rgb(0x2D, 0x7F, 0xF9),
            accent_hover: Color32::from_rgb(0x1C, 0x6B, 0xE0),
            on_accent: Color32::WHITE,
            selected: Color32::from_rgb(0xDC, 0xE9, 0xFE),
            selected_text: Color32::from_rgb(0x12, 0x4F, 0xB3),
            hover: Color32::from_rgb(0xE4, 0xE8, 0xEE),
            pressed: Color32::from_rgb(0xD5, 0xDB, 0xE3),
            input_bg: Color32::WHITE,
            input_border: Color32::from_rgb(0xCF, 0xD4, 0xDB),
            menu_bg: Color32::WHITE,
            shadow: Color32::from_black_alpha(36),
            danger: Color32::from_rgb(0xD8, 0x39, 0x31),
            success: Color32::from_rgb(0x2E, 0xA1, 0x4B),
            flag: Color32::from_rgb(0xE5, 0x48, 0x3E),
            badge: Color32::from_rgb(0x2D, 0x7F, 0xF9),
            chip: Color32::from_rgb(0xF3, 0xF5, 0xF8),
            avatars: AVATARS,
        }
    }

    /// 深色主题。
    pub fn dark() -> Tokens {
        Tokens {
            dark: true,
            toolbar: Color32::from_rgb(0x26, 0x28, 0x2C),
            sidebar: Color32::from_rgb(0x21, 0x23, 0x27),
            list: Color32::from_rgb(0x1B, 0x1C, 0x1F),
            reader: Color32::from_rgb(0x1B, 0x1C, 0x1F),
            status: Color32::from_rgb(0x26, 0x28, 0x2C),
            group: Color32::from_rgb(0x23, 0x25, 0x29),
            separator: Color32::from_rgb(0x36, 0x39, 0x3E),
            text: Color32::from_rgb(0xE6, 0xE8, 0xEB),
            text_dim: Color32::from_rgb(0xA6, 0xAB, 0xB3),
            text_faint: Color32::from_rgb(0x7D, 0x83, 0x8C),
            text_disabled: Color32::from_rgb(0x5A, 0x5F, 0x66),
            accent: Color32::from_rgb(0x4C, 0x8D, 0xFF),
            accent_hover: Color32::from_rgb(0x6A, 0xA1, 0xFF),
            on_accent: Color32::WHITE,
            selected: Color32::from_rgb(0x24, 0x3A, 0x5E),
            selected_text: Color32::from_rgb(0xA9, 0xC8, 0xFF),
            hover: Color32::from_rgb(0x30, 0x33, 0x38),
            pressed: Color32::from_rgb(0x3A, 0x3E, 0x44),
            input_bg: Color32::from_rgb(0x15, 0x16, 0x18),
            input_border: Color32::from_rgb(0x46, 0x4A, 0x51),
            menu_bg: Color32::from_rgb(0x2B, 0x2D, 0x32),
            shadow: Color32::from_black_alpha(110),
            danger: Color32::from_rgb(0xF2, 0x6B, 0x5F),
            success: Color32::from_rgb(0x5B, 0xC2, 0x76),
            flag: Color32::from_rgb(0xF2, 0x5F, 0x55),
            badge: Color32::from_rgb(0x3D, 0x7F, 0xF0),
            chip: Color32::from_rgb(0x2A, 0x2C, 0x31),
            avatars: AVATARS,
        }
    }

    /// 当前上下文的主题颜色。
    pub fn get(ctx: &egui::Context) -> Tokens {
        if ctx.global_style().visuals.dark_mode { Tokens::dark() } else { Tokens::light() }
    }

    /// 按字符串（邮箱）稳定地挑一个头像底色。
    pub fn avatar(&self, seed: &str) -> Color32 {
        // FNV-1a：简单稳定，同一个人每次颜色都一样
        let mut h: u32 = 0x811C_9DC5;
        for b in seed.to_ascii_lowercase().bytes() {
            h ^= u32::from(b);
            h = h.wrapping_mul(0x0100_0193);
        }
        self.avatars.get((h % self.avatars.len() as u32) as usize).copied().unwrap_or(self.accent)
    }
}

/// 常规界面字体族名。
pub const UI: &str = "ui";
/// 粗体界面字体族名（未读发件人、标题）。
pub const UI_BOLD: &str = "ui-bold";

/// 常规界面字体。
pub fn ui_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(UI.into()))
}

/// 粗体界面字体。
pub fn ui_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(UI_BOLD.into()))
}

/// 读一个字体文件，进程内只读一次并常驻（同一个 .ttc 的常规体和粗体共用一份字节）。
#[cfg(not(target_arch = "wasm32"))]
fn load_static(path: &str) -> Option<&'static [u8]> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, &'static [u8]>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mut map = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(b) = map.get(path) {
        return Some(b);
    }
    let bytes = std::fs::read(path).ok()?;
    // 字体是几十 MB 的系统文件，泄漏成 'static 只做一次，避免每次切换主题都复制
    let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    map.insert(path.to_string(), leaked);
    Some(leaked)
}

#[cfg(target_arch = "wasm32")]
fn load_static(_path: &str) -> Option<&'static [u8]> {
    None
}

/// 按顺序尝试候选字体 (路径, 字体索引)，返回第一个能读到的。
fn try_load(paths: &[(String, u32)]) -> Option<FontData> {
    paths.iter().find_map(|(p, index)| {
        let mut fd = FontData::from_static(load_static(p)?);
        fd.index = *index;
        Some(fd)
    })
}

/// macOS 新系统把苹方放在资源目录里（路径带哈希），这里扫一遍找到它。
fn mac_asset_pingfang() -> Option<String> {
    let base = std::path::Path::new("/System/Library/AssetsV2");
    let dirs = std::fs::read_dir(base).ok()?;
    for d in dirs.flatten() {
        if !d.file_name().to_string_lossy().starts_with("com_apple_MobileAsset_Font") {
            continue;
        }
        let Ok(assets) = std::fs::read_dir(d.path()) else { continue };
        for a in assets.flatten() {
            let p = a.path().join("AssetData/PingFang.ttc");
            if p.is_file() {
                return Some(p.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// 中文后备字体候选 (路径, 常规体索引, 粗体索引)，按偏好排序。
fn cjk_candidates() -> Vec<(String, u32, u32)> {
    let win = std::env::var("WINDIR").map(|w| format!("{w}\\Fonts")).unwrap_or_else(|_| "C:\\Windows\\Fonts".into());
    let mut v: Vec<(String, u32, u32)> = vec![("/System/Library/Fonts/PingFang.ttc".into(), 3, 11)];
    if cfg!(target_os = "macos")
        && let Some(p) = mac_asset_pingfang()
    {
        v.push((p, 3, 11));
    }
    v.extend([
        ("/System/Library/Fonts/Hiragino Sans GB.ttc".to_string(), 0, 2),
        ("/System/Library/Fonts/STHeiti Medium.ttc".into(), 1, 1),
        ("/System/Library/Fonts/Supplemental/Songti.ttc".into(), 0, 0),
        ("/System/Library/Fonts/Supplemental/Arial Unicode.ttf".into(), 0, 0),
        (format!("{win}\\msyh.ttc"), 0, 0),
        (format!("{win}\\msyhbd.ttc"), 0, 0),
        (format!("{win}\\simsun.ttc"), 0, 0),
        ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc".into(), 2, 2),
        ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc".into(), 2, 2),
        ("/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc".into(), 2, 2),
        ("/usr/share/fonts/truetype/wqy/wqy-microhei.ttc".into(), 0, 0),
        ("/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc".into(), 0, 0),
        ("/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc".into(), 0, 0),
    ]);
    v
}

/// 拉丁字母界面字体候选：(常规, 粗体)。
fn latin_candidates() -> (Vec<(String, u32)>, Vec<(String, u32)>) {
    let win = std::env::var("WINDIR").map(|w| format!("{w}\\Fonts")).unwrap_or_else(|_| "C:\\Windows\\Fonts".into());
    let lin = ["/usr/share/fonts/truetype", "/usr/share/fonts"];
    let mut regular = vec![
        ("/System/Library/Fonts/SFNS.ttf".to_string(), 0),
        ("/System/Library/Fonts/HelveticaNeue.ttc".into(), 0),
        (format!("{win}\\segoeui.ttf"), 0),
    ];
    let mut bold = vec![("/System/Library/Fonts/HelveticaNeue.ttc".to_string(), 1), (format!("{win}\\segoeuib.ttf"), 0)];
    for d in lin {
        regular.push((format!("{d}/noto/NotoSans-Regular.ttf"), 0));
        regular.push((format!("{d}/dejavu/DejaVuSans.ttf"), 0));
        bold.push((format!("{d}/noto/NotoSans-Bold.ttf"), 0));
        bold.push((format!("{d}/dejavu/DejaVuSans-Bold.ttf"), 0));
    }
    (regular, bold)
}

/// 组装字体表：系统界面字体 → 中文后备 → egui 自带字体（符号、表情）。
pub fn font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let base_prop: Vec<String> = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let base_mono: Vec<String> = fonts.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    let (regular, bold) = latin_candidates();
    let mut ui_chain = Vec::new();
    let mut bold_chain = Vec::new();
    if let Some(fd) = try_load(&regular) {
        fonts.font_data.insert("sys-ui".into(), Arc::new(fd));
        ui_chain.push("sys-ui".to_string());
    }
    if let Some(fd) = try_load(&bold) {
        fonts.font_data.insert("sys-ui-bold".into(), Arc::new(fd));
        bold_chain.push("sys-ui-bold".to_string());
    } else {
        bold_chain.extend(ui_chain.clone());
    }
    // 中文：常规和粗体各取同一文件里的对应字重
    let cjk = cjk_candidates();
    let cjk_regular: Vec<(String, u32)> = cjk.iter().map(|(p, r, _)| (p.clone(), *r)).collect();
    let cjk_bold: Vec<(String, u32)> = cjk.iter().map(|(p, _, b)| (p.clone(), *b)).collect();
    let mut cjk_keys = Vec::new();
    if let Some(fd) = try_load(&cjk_regular) {
        fonts.font_data.insert("cjk".into(), Arc::new(fd));
        ui_chain.push("cjk".into());
        cjk_keys.push("cjk".to_string());
    }
    if let Some(fd) = try_load(&cjk_bold) {
        fonts.font_data.insert("cjk-bold".into(), Arc::new(fd));
        bold_chain.push("cjk-bold".into());
    } else {
        bold_chain.extend(cjk_keys.clone());
    }
    ui_chain.extend(base_prop.clone());
    bold_chain.extend(base_prop);
    let mut mono_chain = Vec::new();
    if let Some(fd) = try_load(&[
        ("/System/Library/Fonts/SFNSMono.ttf".into(), 0),
        ("/System/Library/Fonts/Menlo.ttc".into(), 0),
        ("C:\\Windows\\Fonts\\consola.ttf".into(), 0),
        ("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf".into(), 0),
    ]) {
        fonts.font_data.insert("sys-mono".into(), Arc::new(fd));
        mono_chain.push("sys-mono".to_string());
    }
    mono_chain.extend(base_mono);
    mono_chain.extend(cjk_keys);
    for chain in [&mut ui_chain, &mut bold_chain, &mut mono_chain] {
        chain.dedup();
    }
    fonts.families.insert(FontFamily::Name(UI.into()), ui_chain.clone());
    fonts.families.insert(FontFamily::Name(UI_BOLD.into()), bold_chain);
    fonts.families.insert(FontFamily::Proportional, ui_chain);
    fonts.families.insert(FontFamily::Monospace, mono_chain);
    fonts
}

/// 安装字体（新字体从下一帧起生效）。
pub fn install_fonts(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
}

/// 应用主题颜色、圆角、间距和字号。
pub fn apply(ctx: &egui::Context, dark: bool) {
    let t = if dark { Tokens::dark() } else { Tokens::light() };
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = t.list;
    v.window_fill = t.menu_bg;
    v.window_stroke = Stroke::new(1.0, t.separator);
    v.extreme_bg_color = t.input_bg;
    v.text_edit_bg_color = Some(t.input_bg);
    v.faint_bg_color = t.group;
    v.selection.bg_fill = if dark { Color32::from_rgb(0x2B, 0x4C, 0x80) } else { Color32::from_rgb(0xC5, 0xDB, 0xFD) };
    v.selection.stroke = Stroke::new(1.0, t.accent);
    v.hyperlink_color = t.accent;
    v.override_text_color = None;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.separator);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.inactive.bg_fill = t.chip;
    v.widgets.inactive.weak_bg_fill = t.chip;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.input_border);
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.input_border);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.active.weak_bg_fill = t.pressed;
    v.widgets.active.bg_fill = t.pressed;
    v.widgets.active.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.open.weak_bg_fill = t.pressed;
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = egui::CornerRadius::same(5);
    }
    v.window_corner_radius = egui::CornerRadius::same(10);
    v.menu_corner_radius = egui::CornerRadius::same(8);
    v.window_shadow = egui::epaint::Shadow { offset: [0, 6], blur: 24, spread: 0, color: t.shadow };
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 4], blur: 16, spread: 0, color: t.shadow };
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.menu_margin = egui::Margin::same(6);
        s.text_styles.insert(egui::TextStyle::Body, ui_font(13.5));
        s.text_styles.insert(egui::TextStyle::Button, ui_font(13.5));
        s.text_styles.insert(egui::TextStyle::Small, ui_font(11.5));
        // 标题样式也用于窗口标题栏，所以不能太大
        s.text_styles.insert(egui::TextStyle::Heading, ui_bold(14.5));
        s.text_styles.insert(egui::TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_chains_always_have_egui_defaults() {
        let f = font_definitions();
        for fam in [FontFamily::Name(UI.into()), FontFamily::Name(UI_BOLD.into()), FontFamily::Proportional, FontFamily::Monospace] {
            let chain = f.families.get(&fam).cloned().unwrap_or_default();
            assert!(!chain.is_empty(), "{fam:?}");
            assert!(chain.iter().all(|k| f.font_data.contains_key(k)), "{fam:?}: {chain:?}");
        }
    }

    #[test]
    fn avatar_colour_is_stable() {
        let t = Tokens::light();
        assert_eq!(t.avatar("a@b.c"), t.avatar("A@B.C"));
    }
}
