//! 旧式邮件表格属性的兼容层。UA 层补齐属性，并对 border=0 布局表避开上游折叠边框缺陷。

use std::collections::HashSet;

use blitz_dom::{BaseDocument, local_name};

/// 补齐 blitz 缺失的表格属性，并避免折叠边框把 border-style:none 的默认宽度画成黑框。
pub(crate) fn apply(doc: &mut BaseDocument) {
    // blitz 的折叠边框实现直接读取第一格的 border-width，没有检查 border-style。
    // 默认设为零，作者显式声明的 border/border-width 仍由正常 CSS 级联覆盖。
    let mut css = String::from(":where(table, td, th) { border-width: 0; }\n");
    // 折叠模型还会把第一格的单边装饰画到整张表四周。无 rules 的 border=0 邮件布局表
    // 使用零间距的分离模型，才能保留 td 自己的橙色顶边等样式；数据表格仍保留折叠模型。
    css.push_str("table[border=\"0\"]:not([rules]) { border-collapse: separate !important; border-spacing: 0; }\n");
    let mut rules = HashSet::new();
    if let Ok(ids) = doc.query_selector_all("table") {
        for id in ids {
            let Some(node) = doc.get_node(id) else { continue };
            for name in [local_name!("cellspacing"), local_name!("cellpadding"), local_name!("border")] {
                let Some(value) = node.attr(name.clone()) else { continue };
                // 不把未经校验的邮件属性拼进样式；忽略不合法或过大的旧式像素值。
                if value.is_empty() || value.len() > 6 || !value.bytes().all(|b| b.is_ascii_digit()) {
                    continue;
                }
                let Ok(px) = value.parse::<u32>() else { continue };
                if px > 1000 || !rules.insert((name.clone(), value.to_string())) {
                    continue;
                }
                let selector = format!("table[{name}=\"{value}\"]");
                match name.as_ref() {
                    "cellspacing" => css.push_str(&format!("{selector} {{ border-spacing: {px}px; }}\n")),
                    "cellpadding" => css.push_str(&format!("{selector} > :is(thead,tbody,tfoot) > tr > :is(td,th), {selector} > tr > :is(td,th) {{ padding: {px}px; }}\n")),
                    "border" if px > 0 => css.push_str(&format!("{selector} {{ border: {px}px outset; }} {selector} > :is(thead,tbody,tfoot) > tr > :is(td,th), {selector} > tr > :is(td,th) {{ border: 1px inset; }}\n")),
                    _ => {}
                }
            }
        }
    }
    doc.add_user_agent_stylesheet(&css);
}
