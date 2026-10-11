//! MailFabrica 的图标集：全部用代码在 16×16 设计网格上绘制（原创，无第三方素材）。
//! 所有图标只使用调用方传入的颜色 `c`（及其淡化版本），因此会自动跟随主题。

use egui::epaint::{Mesh, PathShape, PathStroke};
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

/// 旗标红色，调用方在画 Flag 图标时传入。
pub const FLAG_RED: Color32 = Color32::from_rgb(0xE5, 0x48, 0x3E);

/// 邮件客户端用到的全部图标。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Sync,
    Compose,
    Reply,
    ReplyAll,
    Forward,
    Delete,
    Mark,
    Move,
    Junk,
    Search,
    Menu,
    Close,
    ChevronDown,
    ChevronRight,
    ChevronLeft,
    Inbox,
    Drafts,
    Sent,
    Trash,
    JunkFolder,
    Archive,
    Folder,
    AllInboxes,
    Unread,
    Flag,
    FlagOutline,
    Paperclip,
    Plus,
    Contacts,
    Person,
    Settings,
    Info,
    Moon,
    Sun,
    Globe,
    File,
    Save,
    Warning,
    Check,
    Envelope,
    Send,
    Dot,
    /// 项目符号列表。
    ListBullet,
    /// 编号列表。
    ListNumbered,
    /// 引用（左侧竖线）。
    Quote,
    /// 链接（两个相扣的环）。
    Link,
    /// 清除格式。
    ClearFormat,
    /// 文字颜色（字母 A，下面的颜色条由调用方画）。
    TextColor,
    /// 背景色（荧光笔，下面的颜色条由调用方画）。
    Highlight,
    /// 排序/视图选项（三条由长到短的横线）。
    Sort,
}

/// 所有图标变体的列表（测试与图标预览用）。
#[allow(dead_code)]
pub const ALL: [Icon; 50] = [
    Icon::Sync,
    Icon::Compose,
    Icon::Reply,
    Icon::ReplyAll,
    Icon::Forward,
    Icon::Delete,
    Icon::Mark,
    Icon::Move,
    Icon::Junk,
    Icon::Search,
    Icon::Menu,
    Icon::Close,
    Icon::ChevronDown,
    Icon::ChevronRight,
    Icon::ChevronLeft,
    Icon::Inbox,
    Icon::Drafts,
    Icon::Sent,
    Icon::Trash,
    Icon::JunkFolder,
    Icon::Archive,
    Icon::Folder,
    Icon::AllInboxes,
    Icon::Unread,
    Icon::Flag,
    Icon::FlagOutline,
    Icon::Paperclip,
    Icon::Plus,
    Icon::Contacts,
    Icon::Person,
    Icon::Settings,
    Icon::Info,
    Icon::Moon,
    Icon::Sun,
    Icon::Globe,
    Icon::File,
    Icon::Save,
    Icon::Warning,
    Icon::Check,
    Icon::Envelope,
    Icon::Send,
    Icon::Dot,
    Icon::ListBullet,
    Icon::ListNumbered,
    Icon::Quote,
    Icon::Link,
    Icon::ClearFormat,
    Icon::TextColor,
    Icon::Highlight,
    Icon::Sort,
];

/// 设计坐标系中的点（16×16 网格）。
type P = (f32, f32);

/// 绘图上下文：把 16×16 设计网格映射到屏幕矩形。
struct G<'a> {
    p: &'a Painter,
    r: Rect,
    s: f32,
}

/// 在设计坐标系中生成椭圆弧上的采样点；角度单位为度，y 轴向下（90° 即最下方）。
fn arc(cx: f32, cy: f32, rx: f32, ry: f32, a0: f32, a1: f32) -> Vec<P> {
    const N: usize = 10;
    (0..=N)
        .map(|i| {
            let t = i as f32 / N as f32;
            let a = (a0 + (a1 - a0) * t).to_radians();
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}

impl G<'_> {
    /// 设计坐标转屏幕坐标。
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.r.left() + x * self.s, self.r.top() + y * self.s)
    }

    /// 基础线宽：约 1.3 个设计单位，缩放系数限制在 1~2 之间以保证小尺寸清晰、大尺寸不过粗。
    fn sw(&self) -> f32 {
        1.3 * self.s.clamp(1.0, 2.0)
    }

    /// 画一条开放折线。
    fn line(&self, pts: &[P], c: Color32) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.p.add(PathShape::line(v, PathStroke::new(self.sw(), c)));
    }

    /// 画一条闭合折线（描边多边形）。
    fn closed(&self, pts: &[P], c: Color32) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.p.add(PathShape::closed_line(v, PathStroke::new(self.sw(), c)));
    }

    /// 画矩形描边（轻微圆角）。
    fn rect(&self, x: f32, y: f32, w: f32, h: f32, c: Color32) {
        let r = Rect::from_min_size(self.pt(x, y), vec2(w * self.s, h * self.s));
        self.p.rect_stroke(r, 1.0, Stroke::new(self.sw(), c), StrokeKind::Middle);
    }

    /// 画圆描边。
    fn circle(&self, x: f32, y: f32, r: f32, c: Color32) {
        self.p.circle_stroke(self.pt(x, y), r * self.s, Stroke::new(self.sw(), c));
    }

    /// 画实心圆。
    fn dot(&self, x: f32, y: f32, r: f32, c: Color32) {
        self.p.circle_filled(self.pt(x, y), r * self.s, c);
    }

    /// 用三角形网格填充（可填充凹多边形；网格无羽化，边缘需由调用方再描边抗锯齿）。
    fn fill_tris(&self, tris: &[[P; 3]], c: Color32) {
        let mut m = Mesh::default();
        for tri in tris {
            let base = m.vertices.len() as u32;
            for &(x, y) in tri {
                m.colored_vertex(self.pt(x, y), c);
            }
            m.add_triangle(base, base + 1, base + 2);
        }
        self.p.add(Shape::mesh(m));
    }

    /// 画向左（flip 时向右）的弯曲回复箭头：箭头尖在 (hx,hy)，向右延伸 len 后以半径 3 向下拐弯，再下垂 drop。
    fn arrow_back(&self, hx: f32, hy: f32, len: f32, drop: f32, c: Color32, flip: bool) {
        const R: f32 = 3.0;
        // 翻转时对 x 做镜像，复用同一份几何
        let fx = |p: P| -> P { if flip { (16.0 - p.0, p.1) } else { p } };
        let head: Vec<P> = [(hx + R, hy - R), (hx, hy), (hx + R, hy + R)].into_iter().map(fx).collect();
        let mut body: Vec<P> = vec![(hx, hy)];
        body.extend(arc(hx + len, hy + R, R, R, -90.0, 0.0));
        body.push((hx + len + R, hy + R + drop));
        let body: Vec<P> = body.into_iter().map(fx).collect();
        self.line(&head, c);
        self.line(&body, c);
    }

    /// 画文件夹轮廓（左上带标签凸起）。
    fn folder_outline(&self, c: Color32) {
        self.closed(&[(1.5, 13.0), (1.5, 3.0), (6.0, 3.0), (7.5, 4.8), (14.5, 4.8), (14.5, 13.0)], c);
    }

    /// 画折角文件轮廓。
    fn sheet(&self, c: Color32) {
        self.closed(&[(3.5, 1.8), (9.5, 1.8), (12.5, 4.8), (12.5, 14.2), (3.5, 14.2)], c);
        self.line(&[(9.5, 1.8), (9.5, 4.8), (12.5, 4.8)], c);
    }

    /// 画信封：矩形加 V 形封口。
    fn envelope(&self, x: f32, y: f32, w: f32, h: f32, c: Color32) {
        self.rect(x, y, w, h, c);
        self.line(&[(x + 0.3, y + 0.7), (x + w / 2.0, y + h * 0.58), (x + w - 0.3, y + 0.7)], c);
    }

    /// 画纸飞机的两个三角形（凹四边形拆成两个凸三角形）。
    fn plane_tris() -> [[P; 3]; 2] {
        [[(1.8, 7.5), (14.2, 2.0), (7.5, 9.2)], [(14.2, 2.0), (10.5, 14.0), (7.5, 9.2)]]
    }

    /// 画齿轮：8 个齿的外轮廓加中心圆孔。
    fn gear(&self, c: Color32) {
        let mut pts: Vec<P> = Vec::with_capacity(32);
        for k in 0..8 {
            let base = k as f32 * 45.0;
            // 每个齿：内圈起点 -> 外圈两点 -> 内圈终点
            for (da, r) in [(-17.0_f32, 4.7_f32), (-10.0, 6.6), (10.0, 6.6), (17.0, 4.7)] {
                let a = (base + da).to_radians();
                pts.push((8.0 + r * a.cos(), 8.0 + r * a.sin()));
            }
        }
        self.closed(&pts, c);
        self.circle(8.0, 8.0, 2.1, c);
    }

    /// 画回形针：由直线段与半圆弧拼成的单条折线。
    fn paperclip(&self, c: Color32) {
        let mut v: Vec<P> = vec![(6.8, 11.0)];
        // 内侧自由端向上，顶部小弧拐到第二列
        v.extend(arc(8.2, 5.5, 1.4, 1.4, 180.0, 360.0));
        // 第二列向下，底部大弧拐到最外左列
        v.extend(arc(7.1, 11.0, 2.5, 2.5, 0.0, 180.0));
        // 最外左列向上，顶部外弧拐到最外右列并下垂
        v.extend(arc(8.2, 4.8, 3.6, 3.6, 180.0, 360.0));
        v.push((11.8, 10.0));
        self.line(&v, c);
    }

    /// 画胶囊形闭合轮廓：中心 (cx,cy)，沿 `deg` 方向半长 `l`（直线段的一半），两端半圆半径 `r`。
    fn capsule(&self, cx: f32, cy: f32, l: f32, r: f32, deg: f32, c: Color32) {
        let a = deg.to_radians();
        let (dx, dy) = (a.cos() * l, a.sin() * l);
        // 先画朝向 deg 的那一端半圆，再画另一端，首尾自动闭合
        let mut pts = arc(cx + dx, cy + dy, r, r, deg - 90.0, deg + 90.0);
        pts.extend(arc(cx - dx, cy - dy, r, r, deg + 90.0, deg + 270.0));
        self.closed(&pts, c);
    }

    /// 画月牙：外圆减去偏移的内圆，几何交点按公式计算，填充后再描边抗锯齿。
    fn moon(&self, c: Color32) {
        let (c0, r0) = ((8.0_f32, 8.0_f32), 6.3_f32);
        let (c1, r1) = ((11.2_f32, 5.4_f32), 5.2_f32);
        let (dx, dy) = (c1.0 - c0.0, c1.1 - c0.1);
        let d = (dx * dx + dy * dy).sqrt().max(0.001);
        let dir = dy.atan2(dx);
        // a：两圆连心线上，外圆圆心到公共弦的距离
        let a = (r0 * r0 - r1 * r1 + d * d) / (2.0 * d);
        let phi = (a / r0).clamp(-1.0, 1.0).acos();
        let psi = ((d - a) / r1).clamp(-1.0, 1.0).acos();
        const N: usize = 14;
        let mut outer: Vec<P> = Vec::with_capacity(N + 1);
        let mut inner: Vec<P> = Vec::with_capacity(N + 1);
        for i in 0..=N {
            let t = i as f32 / N as f32;
            // 外圆取远离内圆的长弧；内圆取落在外圆内的短弧，两者从同一交点出发
            let ao = dir + phi + t * (std::f32::consts::TAU - 2.0 * phi);
            let ai = dir + std::f32::consts::PI - psi + t * 2.0 * psi;
            outer.push((c0.0 + r0 * ao.cos(), c0.1 + r0 * ao.sin()));
            inner.push((c1.0 + r1 * ai.cos(), c1.1 + r1 * ai.sin()));
        }
        let mut tris: Vec<[P; 3]> = Vec::with_capacity(N * 2);
        for (o, n) in outer.windows(2).zip(inner.windows(2)) {
            if let (Some(&o0), Some(&o1), Some(&n0), Some(&n1)) = (o.first(), o.get(1), n.first(), n.get(1)) {
                tris.push([o0, o1, n1]);
                tris.push([o0, n1, n0]);
            }
        }
        self.fill_tris(&tris, c);
        let mut outline = outer;
        outline.extend(inner.into_iter().rev());
        self.closed(&outline, c);
    }
}

/// 在 rect（取中间的正方形）里用颜色 c 画图标。
pub fn paint(p: &Painter, rect: Rect, icon: Icon, c: Color32) {
    let side = rect.width().min(rect.height());
    let r = Rect::from_center_size(rect.center(), vec2(side, side));
    let g = G { p, r, s: side / 16.0 };
    let light = c.gamma_multiply(0.55);
    match icon {
        Icon::Sync => {
            // 收取：向下箭头落入敞口托盘
            g.line(&[(8.0, 2.0), (8.0, 9.8)], c);
            g.line(&[(4.8, 6.8), (8.0, 10.0), (11.2, 6.8)], c);
            g.line(&[(2.5, 10.0), (2.5, 13.5), (13.5, 13.5), (13.5, 10.0)], c);
        }
        Icon::Compose => {
            // 纸张（右上留给铅笔）+ 斜置铅笔
            g.line(&[(8.0, 2.5), (3.0, 2.5), (3.0, 13.5), (12.0, 13.5), (12.0, 9.5)], c);
            g.line(&[(5.5, 6.0), (7.0, 6.0)], light);
            g.line(&[(5.5, 9.0), (7.5, 9.0)], light);
            g.fill_tris(&[[(8.0, 6.0), (10.0, 8.0), (7.0, 9.0)]], c);
            g.line(&[(9.0, 7.0), (12.7, 3.3)], c);
            g.line(&[(9.4, 7.4), (13.1, 3.7)], c);
            g.line(&[(11.9, 2.4), (13.6, 4.1)], c);
        }
        Icon::Reply => g.arrow_back(3.0, 6.5, 6.0, 2.5, c, false),
        Icon::ReplyAll => {
            g.arrow_back(6.0, 4.5, 4.5, 1.5, light, false);
            g.arrow_back(1.8, 8.5, 5.0, 1.5, c, false);
        }
        Icon::Forward => g.arrow_back(3.0, 6.5, 6.0, 2.5, c, true),
        Icon::Delete | Icon::Trash => {
            g.line(&[(2.5, 4.0), (13.5, 4.0)], c);
            g.line(&[(6.0, 4.0), (6.0, 2.5), (10.0, 2.5), (10.0, 4.0)], c);
            g.line(&[(3.8, 4.5), (4.6, 13.5), (11.4, 13.5), (12.2, 4.5)], c);
            g.line(&[(6.6, 7.0), (6.6, 11.0)], light);
            g.line(&[(9.4, 7.0), (9.4, 11.0)], light);
        }
        Icon::Mark => {
            // 标签：右侧尖角的五边形加挂孔
            g.closed(&[(2.5, 4.0), (10.5, 4.0), (14.0, 8.0), (10.5, 12.0), (2.5, 12.0)], c);
            g.dot(10.0, 8.0, 1.0, c);
        }
        Icon::Move => {
            g.folder_outline(c);
            g.line(&[(4.5, 9.0), (10.8, 9.0)], c);
            g.line(&[(8.5, 6.8), (10.8, 9.0), (8.5, 11.2)], c);
        }
        Icon::Junk | Icon::JunkFolder => {
            // 盾牌里一个叉
            g.closed(&[(8.0, 1.8), (13.0, 3.6), (13.0, 7.5), (11.8, 10.5), (8.0, 14.0), (4.2, 10.5), (3.0, 7.5), (3.0, 3.6)], c);
            g.line(&[(6.1, 5.6), (9.9, 9.4)], c);
            g.line(&[(9.9, 5.6), (6.1, 9.4)], c);
        }
        Icon::Search => {
            g.circle(6.8, 6.8, 4.6, c);
            g.line(&[(10.2, 10.2), (14.0, 14.0)], c);
        }
        Icon::Menu => {
            for y in [4.0, 8.0, 12.0] {
                g.line(&[(2.5, y), (13.5, y)], c);
            }
        }
        Icon::Sort => {
            for (y, end) in [(4.0, 13.5), (8.0, 10.5), (12.0, 7.5)] {
                g.line(&[(2.5, y), (end, y)], c);
            }
        }
        Icon::Close => {
            g.line(&[(3.5, 3.5), (12.5, 12.5)], c);
            g.line(&[(12.5, 3.5), (3.5, 12.5)], c);
        }
        Icon::ChevronDown => g.line(&[(4.5, 6.2), (8.0, 9.8), (11.5, 6.2)], c),
        Icon::ChevronRight => g.line(&[(6.2, 4.5), (9.8, 8.0), (6.2, 11.5)], c),
        Icon::ChevronLeft => g.line(&[(9.8, 4.5), (6.2, 8.0), (9.8, 11.5)], c),
        Icon::Inbox => {
            g.closed(&[(2.0, 9.5), (4.2, 3.0), (11.8, 3.0), (14.0, 9.5), (14.0, 13.0), (2.0, 13.0)], c);
            g.line(&[(2.0, 9.5), (5.5, 9.5), (6.5, 11.0), (9.5, 11.0), (10.5, 9.5), (14.0, 9.5)], c);
        }
        Icon::Drafts => {
            g.sheet(c);
            g.line(&[(5.7, 8.0), (10.3, 8.0)], light);
            g.line(&[(5.7, 11.0), (9.0, 11.0)], light);
        }
        Icon::Sent => {
            g.closed(&[(1.8, 7.5), (14.2, 2.0), (10.5, 14.0), (7.5, 9.2)], c);
            g.line(&[(7.5, 9.2), (14.2, 2.0)], c);
        }
        Icon::Archive => {
            g.rect(1.8, 2.8, 12.4, 3.2, c);
            g.line(&[(3.0, 6.0), (3.0, 13.2), (13.0, 13.2), (13.0, 6.0)], c);
            g.line(&[(6.5, 9.0), (9.5, 9.0)], c);
        }
        Icon::Folder => g.folder_outline(c),
        Icon::AllInboxes => {
            g.line(&[(3.5, 6.5), (5.0, 2.0), (11.0, 2.0), (12.5, 6.5)], light);
            g.closed(&[(1.5, 10.0), (3.2, 6.5), (12.8, 6.5), (14.5, 10.0), (14.5, 13.5), (1.5, 13.5)], c);
            g.line(&[(1.5, 10.0), (5.5, 10.0), (6.5, 11.5), (9.5, 11.5), (10.5, 10.0), (14.5, 10.0)], c);
        }
        Icon::Unread => {
            g.envelope(1.5, 5.0, 11.0, 8.5, c);
            g.dot(12.8, 3.4, 2.3, c);
        }
        Icon::Flag => {
            g.line(&[(4.0, 2.0), (4.0, 14.5)], c);
            let tri = [(4.0, 2.5), (13.5, 5.75), (4.0, 9.0)];
            g.fill_tris(&[tri], c);
            g.closed(&tri, c);
        }
        Icon::FlagOutline => {
            g.line(&[(4.0, 2.0), (4.0, 14.5)], c);
            g.closed(&[(4.0, 2.5), (13.5, 5.75), (4.0, 9.0)], c);
        }
        Icon::Paperclip => g.paperclip(c),
        Icon::Plus => {
            g.line(&[(8.0, 3.0), (8.0, 13.0)], c);
            g.line(&[(3.0, 8.0), (13.0, 8.0)], c);
        }
        Icon::Contacts => {
            // 通讯录：书本 + 人像 + 右侧索引页签
            g.rect(3.0, 1.8, 9.5, 12.4, c);
            g.circle(7.75, 6.3, 1.7, c);
            g.line(&arc(7.75, 12.0, 2.9, 2.9, 180.0, 360.0), c);
            for y in [4.5, 8.0, 11.5] {
                g.line(&[(12.5, y), (14.2, y)], c);
            }
        }
        Icon::Person => {
            g.circle(8.0, 5.0, 2.6, c);
            g.line(&arc(8.0, 14.0, 5.5, 5.0, 180.0, 360.0), c);
        }
        Icon::Settings => g.gear(c),
        Icon::Info => {
            g.circle(8.0, 8.0, 6.2, c);
            g.dot(8.0, 5.2, 0.9, c);
            g.line(&[(8.0, 7.4), (8.0, 11.2)], c);
        }
        Icon::Moon => g.moon(c),
        Icon::Sun => {
            g.circle(8.0, 8.0, 2.9, c);
            for k in 0..8 {
                let a = (k as f32 * 45.0).to_radians();
                let (cs, sn) = (a.cos(), a.sin());
                g.line(&[(8.0 + 4.9 * cs, 8.0 + 4.9 * sn), (8.0 + 6.8 * cs, 8.0 + 6.8 * sn)], c);
            }
        }
        Icon::Globe => {
            g.circle(8.0, 8.0, 6.2, c);
            g.closed(&arc(8.0, 8.0, 2.7, 6.2, 0.0, 360.0), c);
            g.line(&[(1.8, 8.0), (14.2, 8.0)], c);
            g.line(&[(2.5, 5.0), (13.5, 5.0)], light);
            g.line(&[(2.5, 11.0), (13.5, 11.0)], light);
        }
        Icon::File => g.sheet(c),
        Icon::Save => {
            g.line(&[(8.0, 2.0), (8.0, 10.0)], c);
            g.line(&[(4.8, 7.0), (8.0, 10.2), (11.2, 7.0)], c);
            g.line(&[(3.0, 13.5), (13.0, 13.5)], c);
        }
        Icon::Warning => {
            g.closed(&[(8.0, 2.0), (14.5, 13.5), (1.5, 13.5)], c);
            g.line(&[(8.0, 6.2), (8.0, 9.6)], c);
            g.dot(8.0, 11.7, 0.85, c);
        }
        Icon::Check => g.line(&[(3.0, 8.5), (6.5, 12.0), (13.0, 4.5)], c),
        Icon::Envelope => g.envelope(1.5, 3.5, 13.0, 9.0, c),
        Icon::Send => {
            g.fill_tris(&G::plane_tris(), c);
            g.closed(&[(1.8, 7.5), (14.2, 2.0), (10.5, 14.0), (7.5, 9.2)], c);
        }
        Icon::Dot => g.dot(8.0, 8.0, 4.0, c),
        Icon::ListBullet => {
            for y in [3.5, 8.0, 12.5] {
                g.dot(3.0, y, 1.25, c);
                g.line(&[(6.5, y), (14.0, y)], c);
            }
        }
        Icon::ListNumbered => {
            // 三个小数字 1、2、3（折线写法）加三条横线
            g.line(&[(2.2, 2.6), (3.4, 1.9), (3.4, 5.2)], c);
            g.line(&[(2.0, 7.0), (2.6, 6.4), (3.8, 6.4), (4.3, 7.0), (2.0, 9.6), (4.5, 9.6)], c);
            g.line(&[(2.0, 11.0), (4.3, 11.0), (3.1, 12.3), (4.4, 12.9), (4.3, 14.0), (3.6, 14.4), (2.0, 14.2)], c);
            for y in [3.5, 8.0, 12.5] {
                g.line(&[(6.5, y), (14.0, y)], c);
            }
        }
        Icon::Quote => {
            g.fill_tris(&[[(2.0, 2.5), (3.6, 2.5), (3.6, 13.5)], [(2.0, 2.5), (3.6, 13.5), (2.0, 13.5)]], c);
            g.line(&[(6.5, 4.0), (14.0, 4.0)], c);
            g.line(&[(6.5, 8.0), (14.0, 8.0)], light);
            g.line(&[(6.5, 12.0), (12.0, 12.0)], light);
        }
        Icon::Link => {
            g.capsule(5.9, 10.1, 1.9, 2.2, -45.0, c);
            g.capsule(10.1, 5.9, 1.9, 2.2, -45.0, c);
        }
        Icon::ClearFormat => {
            g.line(&[(2.0, 3.0), (11.0, 3.0)], c);
            g.line(&[(6.5, 3.0), (6.5, 13.5)], c);
            g.line(&[(10.0, 9.5), (14.0, 13.5)], c);
            g.line(&[(14.0, 9.5), (10.0, 13.5)], c);
        }
        Icon::TextColor => {
            g.line(&[(3.6, 11.0), (8.0, 1.6), (12.4, 11.0)], c);
            g.line(&[(5.3, 7.6), (10.7, 7.6)], c);
        }
        Icon::Highlight => {
            // 斜放的荧光笔：笔身 + 笔尖
            g.closed(&[(9.2, 1.6), (13.4, 5.8), (8.2, 11.0), (4.0, 6.8)], c);
            g.closed(&[(4.0, 6.8), (8.2, 11.0), (5.6, 12.0), (3.0, 9.4)], c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 把每个图标画进 painter，确认不会 panic。
    #[test]
    fn paint_all_icons_without_panic() {
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(24.0, 24.0));
        let mut out = ctx.run_ui(Default::default(), |ui| {
            for icon in ALL {
                paint(ui.painter(), rect, icon, Color32::BLACK);
            }
        });
        // 测试里没有渲染器，纹理更新要手动丢掉
        out.textures_delta.clear();
    }
}
