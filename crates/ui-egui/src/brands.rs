//! 添加账户时邮箱类型的厂商标志：全部用代码画的简化版（不放图片文件），颜色取各家品牌色。
//! 这些标志用固定颜色，不跟随主题；底下统一垫一块浅色圆角底，深色模式下也看得清。

use egui::epaint::PathStroke;
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

use crate::icons::{self, Icon};

/// 品牌色。
const QQ_BLUE: Color32 = Color32::from_rgb(0x12, 0xB7, 0xF5);
const EXMAIL_BLUE: Color32 = Color32::from_rgb(0x2D, 0x7C, 0xF6);
const NETEASE_RED: Color32 = Color32::from_rgb(0xD8, 0x28, 0x2F);
const ALI_ORANGE: Color32 = Color32::from_rgb(0xFF, 0x6A, 0x00);
const G_BLUE: Color32 = Color32::from_rgb(0x42, 0x85, 0xF4);
const G_RED: Color32 = Color32::from_rgb(0xEA, 0x43, 0x35);
const G_YELLOW: Color32 = Color32::from_rgb(0xFB, 0xBC, 0x04);
const G_GREEN: Color32 = Color32::from_rgb(0x34, 0xA8, 0x53);
const ICLOUD_BLUE: Color32 = Color32::from_rgb(0x36, 0x93, 0xF3);
const OTHER_GRAY: Color32 = Color32::from_rgb(0x8A, 0x9B, 0xB0);

/// 在 `rect`（正方形）里画邮箱类型 `id` 的标志；不认识的 id 画"其他邮箱"的信封。
pub fn paint(p: &Painter, rect: Rect, id: &str) {
    let r = rect.width().min(rect.height()) / 2.0;
    let c = rect.center();
    // 0..1 的设计坐标 → 屏幕坐标
    let at = |x: f32, y: f32| pos2(c.x + (x - 0.5) * 2.0 * r, c.y + (y - 0.5) * 2.0 * r);
    let w = |k: f32| k * 2.0 * r;
    let tile = |fill: Color32| p.rect_filled(rect, w(0.24), fill);
    match id {
        "qq" => {
            // 企鹅：黑身白肚、两只眼睛、黄嘴、红围巾、黄脚
            tile(Color32::from_rgb(0xE8, 0xF7, 0xFE));
            let black = Color32::from_rgb(0x1F, 0x1F, 0x1F);
            p.add(Shape::ellipse_filled(at(0.5, 0.52), vec2(w(0.25), w(0.32)), black));
            p.add(Shape::ellipse_filled(at(0.5, 0.6), vec2(w(0.17), w(0.21)), Color32::WHITE));
            for x in [0.43, 0.57] {
                p.circle_filled(at(x, 0.36), w(0.055), Color32::WHITE);
                p.circle_filled(at(x + 0.01, 0.365), w(0.025), black);
            }
            p.add(Shape::ellipse_filled(at(0.5, 0.445), vec2(w(0.07), w(0.035)), G_YELLOW));
            p.add(Shape::line(vec![at(0.3, 0.5), at(0.5, 0.56), at(0.7, 0.5)], PathStroke::new(w(0.06), G_RED)));
            for x in [0.4, 0.6] {
                p.add(Shape::ellipse_filled(at(x, 0.84), vec2(w(0.09), w(0.04)), G_YELLOW));
            }
            // 右下角一个小信封角标，表示"邮箱"
            let badge = Rect::from_center_size(at(0.8, 0.8), vec2(w(0.3), w(0.3)));
            p.rect_filled(badge, w(0.07), QQ_BLUE);
            icons::paint(p, badge.shrink(w(0.04)), Icon::Envelope, Color32::WHITE);
        }
        "exmail" => {
            // 蓝色"蝴蝶结"：左右两个方括号，中间交叉
            tile(Color32::from_rgb(0xEA, 0xF2, 0xFE));
            let s = PathStroke::new(w(0.09), EXMAIL_BLUE);
            p.add(Shape::line(vec![at(0.36, 0.3), at(0.2, 0.3), at(0.2, 0.7), at(0.36, 0.7)], s.clone()));
            p.add(Shape::line(vec![at(0.64, 0.3), at(0.8, 0.3), at(0.8, 0.7), at(0.64, 0.7)], s.clone()));
            p.add(Shape::line(vec![at(0.36, 0.3), at(0.64, 0.7)], s.clone()));
            p.add(Shape::line(vec![at(0.36, 0.7), at(0.64, 0.3)], s));
        }
        "netease" | "qiye163" => {
            // 网易红底白信封；企业版右下角加一个"企"字角标
            tile(NETEASE_RED);
            let env = Rect::from_center_size(at(0.5, 0.5), vec2(w(0.6), w(0.6)));
            icons::paint(p, env, Icon::Envelope, Color32::WHITE);
            if id == "qiye163" {
                corner_text(p, at(0.8, 0.8), w(0.32), "企", NETEASE_RED);
            }
        }
        "aliqiye" => {
            // 橙底白色"[—]"：左右括号加中间一横
            tile(ALI_ORANGE);
            let s = PathStroke::new(w(0.085), Color32::WHITE);
            p.add(Shape::line(vec![at(0.38, 0.3), at(0.24, 0.3), at(0.24, 0.7), at(0.38, 0.7)], s.clone()));
            p.add(Shape::line(vec![at(0.62, 0.3), at(0.76, 0.3), at(0.76, 0.7), at(0.62, 0.7)], s.clone()));
            p.add(Shape::line(vec![at(0.4, 0.5), at(0.6, 0.5)], s));
            corner_text(p, at(0.8, 0.8), w(0.32), "企", ALI_ORANGE);
        }
        "gmail" => {
            // 四色 M 信封：左蓝、右绿、中间红色 V、右上黄
            tile(Color32::WHITE);
            p.rect_stroke(rect, w(0.24), Stroke::new(1.0, Color32::from_rgb(0xE3, 0xE6, 0xEA)), StrokeKind::Inside);
            let (l, rt, t, b) = (0.2, 0.8, 0.3, 0.72);
            let k = w(0.1);
            p.add(Shape::line(vec![at(l, b), at(l, t)], PathStroke::new(k, G_BLUE)));
            p.add(Shape::line(vec![at(rt, b), at(rt, t)], PathStroke::new(k, G_GREEN)));
            p.add(Shape::line(vec![at(l, t), at(0.5, 0.52)], PathStroke::new(k, G_RED)));
            p.add(Shape::line(vec![at(0.5, 0.52), at(rt, t)], PathStroke::new(k, G_YELLOW)));
            for (pt, col) in [(at(l, t), G_RED), (at(rt, t), G_YELLOW)] {
                p.circle_filled(pt, k / 2.0, col);
            }
        }
        "icloud" => {
            // 蓝色云朵：几个圆叠成云，底边拉平
            tile(Color32::from_rgb(0xEB, 0xF4, 0xFE));
            for (x, y, rr) in [(0.36, 0.55, 0.14), (0.5, 0.45, 0.18), (0.65, 0.53, 0.15)] {
                p.circle_filled(at(x, y), w(rr), ICLOUD_BLUE);
            }
            p.rect_filled(Rect::from_min_max(at(0.36, 0.52), at(0.65, 0.68)), 0.0, ICLOUD_BLUE);
        }
        _ => {
            tile(Color32::from_rgb(0xEE, 0xF1, 0xF5));
            let env = Rect::from_center_size(at(0.5, 0.5), vec2(w(0.6), w(0.6)));
            icons::paint(p, env, Icon::Envelope, OTHER_GRAY);
        }
    }
}

/// 右下角的白底圆形小字角标（"企"）。
fn corner_text(p: &Painter, center: Pos2, size: f32, text: &str, color: Color32) {
    p.circle_filled(center, size / 2.0, Color32::WHITE);
    p.text(center, egui::Align2::CENTER_CENTER, text, crate::theme::ui_bold(size * 0.62), color);
}
