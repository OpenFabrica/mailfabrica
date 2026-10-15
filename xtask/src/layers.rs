//! `cargo xtask layers`：crate 依赖分层规则。
//!
//! 规则引擎只处理一个与 cargo metadata 无关的小模型，方便单元测试；
//! `from_metadata` 负责把 `cargo metadata` 的输出转成这个模型。

use serde_json::Value;

/// 一个 workspace crate 所处的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// 普通分层 crate。
    Layer(u8),
    /// 构建工具：不受规则约束。
    Exempt,
}

impl Class {
    /// 层号；豁免的返回 None。
    fn layer(self) -> Option<u8> {
        match self {
            Class::Layer(l) => Some(l),
            Class::Exempt => None,
        }
    }
}

/// 分层表。名字是去掉 `mailfabrica-` 前缀的包名。同层之间不能互相依赖
/// （所以 mime 和 proto、ui-egui 和 mcp 互相独立）。
pub const TABLE: &[(&str, Class)] = &[
    ("core", Class::Layer(0)),
    ("brand", Class::Layer(0)),
    ("mime", Class::Layer(1)),
    ("proto", Class::Layer(1)),
    ("rich", Class::Layer(1)),
    ("store", Class::Layer(2)),
    ("sync", Class::Layer(3)),
    ("update", Class::Layer(3)),
    ("ai", Class::Layer(3)),
    ("html", Class::Layer(4)),
    ("engine", Class::Layer(5)),
    ("ui-egui", Class::Layer(6)),
    ("mcp", Class::Layer(6)),
    // L7 应用（不检查）和工具
    ("mailfabrica", Class::Layer(7)),
    ("cli", Class::Layer(7)),
    ("xtask", Class::Exempt),
];

/// 层内的单向顺序（前面的可以被后面的使用）。目前没有，保留给以后。
pub const INTRA_LAYER_ORDER: &[&[&str]] = &[];

/// 同层内 `from` 依赖 `to` 是否被显式允许。
fn intra_layer_allowed(from: &str, to: &str) -> bool {
    let (from, to) = (short_name(from), short_name(to));
    INTRA_LAYER_ORDER.iter().any(|chain| match (chain.iter().position(|n| *n == from), chain.iter().position(|n| *n == to)) {
        (Some(f), Some(t)) => t < f,
        _ => false,
    })
}

/// 属于 UI 工具包 / 窗口系统的外部 crate。以 `*` 结尾的是前缀。
pub const UI_CRATES: &[&str] = &["egui*", "eframe", "winit", "rfd", "wgpu*"];

/// 允许使用 UI crate 的最低层。
pub const UI_MIN_LAYER: u8 = 6;

/// 应用层：依赖任何东西都可以。
const APP_LAYER: u8 = 7;

/// 去掉 `mailfabrica-` 前缀的包名。
pub fn short_name(pkg: &str) -> &str {
    pkg.strip_prefix("mailfabrica-").unwrap_or(pkg)
}

/// 查表得到 crate 的分类；表里没有返回 None。
pub fn classify(pkg: &str) -> Option<Class> {
    let s = short_name(pkg);
    TABLE.iter().find(|(n, _)| *n == s).map(|(_, c)| *c)
}

/// 是否 UI 工具包 crate。
fn is_ui_crate(name: &str) -> bool {
    UI_CRATES.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == *p,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

/// 一条依赖。
#[derive(Debug, Clone)]
pub struct Dep {
    pub name: String,
    pub kind: DepKind,
    /// 是否 workspace 成员。
    pub workspace: bool,
}

/// 一个 crate 及其依赖。
#[derive(Debug, Clone)]
pub struct Crate {
    pub name: String,
    pub deps: Vec<Dep>,
}

/// 违反的规则。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    Unregistered { krate: String },
    Upward { krate: String, dep: String, from: u8, to: u8, kind: DepKind },
    UiBelowL6 { krate: String, dep: String, layer: u8 },
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Violation::Unregistered { krate } => {
                write!(f, "{krate}: unknown workspace crate; register it in xtask/src/layers.rs TABLE")
            }
            Violation::Upward { krate, dep, from, to, kind } => {
                write!(f, "{krate} (L{from}) -> {dep} (L{to}) [{kind:?}]: may only depend on strictly lower layers")
            }
            Violation::UiBelowL6 { krate, dep, layer } => {
                write!(f, "{krate} (L{layer}) depends on UI crate `{dep}`; UI toolkits are only allowed in L{UI_MIN_LAYER}+")
            }
        }
    }
}

/// 检查全部规则，返回排好序的违规列表（输出稳定）。
pub fn check(crates: &[Crate]) -> Vec<Violation> {
    let mut out = Vec::new();
    for c in crates {
        let Some(class) = classify(&c.name) else {
            out.push(Violation::Unregistered { krate: c.name.clone() });
            continue;
        };
        // 工具和 L7 应用站在一切之上。
        let Some(layer) = class.layer().filter(|l| *l < APP_LAYER) else {
            continue;
        };
        for d in &c.deps {
            // 自己对自己的 dev 依赖（测试里开 feature）没问题。
            if d.name == c.name {
                continue;
            }
            if d.workspace {
                // 未注册的依赖在它自己的条目里报告。
                let Some(dc) = classify(&d.name) else { continue };
                let to = dc.layer().unwrap_or(u8::MAX);
                if to >= layer && !(to == layer && intra_layer_allowed(&c.name, &d.name)) {
                    out.push(Violation::Upward {
                        krate: c.name.clone(),
                        dep: d.name.clone(),
                        from: layer,
                        to: if to == u8::MAX { APP_LAYER } else { to },
                        kind: d.kind,
                    });
                }
            } else if layer < UI_MIN_LAYER && is_ui_crate(&d.name) {
                out.push(Violation::UiBelowL6 { krate: c.name.clone(), dep: d.name.clone(), layer });
            }
        }
    }
    out.sort_by_key(|v| v.to_string());
    out.dedup();
    out
}

/// 从 `cargo metadata --format-version 1 --no-deps` 的输出构造模型。
pub fn from_metadata(meta: &Value) -> Result<Vec<Crate>, String> {
    let pkgs = meta["packages"].as_array().ok_or("metadata: no packages array")?;
    let members: Vec<&str> = pkgs.iter().filter_map(|p| p["name"].as_str()).collect();
    let mut out = Vec::new();
    for p in pkgs {
        let name = p["name"].as_str().ok_or("package without name")?.to_owned();
        let mut deps = Vec::new();
        for d in p["dependencies"].as_array().into_iter().flatten() {
            let dname = d["name"].as_str().unwrap_or_default().to_owned();
            let kind = match d["kind"].as_str() {
                Some("dev") => DepKind::Dev,
                Some("build") => DepKind::Build,
                _ => DepKind::Normal,
            };
            let workspace = members.contains(&dname.as_str()) || d["path"].is_string();
            deps.push(Dep { name: dname, kind, workspace });
        }
        out.push(Crate { name, deps });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// 层的人类可读描述。
pub fn describe(class: Option<Class>) -> String {
    match class {
        Some(Class::Layer(l)) if l == APP_LAYER => "L7 app".into(),
        Some(Class::Layer(l)) => format!("L{l}"),
        Some(Class::Exempt) => "unchecked".into(),
        None => "UNREGISTERED".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DepKind::*;

    /// 造一个测试用 crate。
    fn c(name: &str, deps: &[(&str, DepKind, bool)]) -> Crate {
        Crate { name: name.into(), deps: deps.iter().map(|(n, k, w)| Dep { name: (*n).into(), kind: *k, workspace: *w }).collect() }
    }

    #[test]
    fn clean_downward_graph_passes() {
        let g = [
            c("mailfabrica-core", &[("serde", Normal, false)]),
            c("mailfabrica-mime", &[("mailfabrica-core", Normal, true), ("mail-parser", Normal, false)]),
            c("mailfabrica-proto", &[("mailfabrica-core", Normal, true), ("rustls", Normal, false)]),
            c("mailfabrica-store", &[("mailfabrica-core", Normal, true), ("mailfabrica-mime", Normal, true)]),
            c("mailfabrica-sync", &[("mailfabrica-proto", Normal, true), ("mailfabrica-store", Normal, true), ("mailfabrica-mime", Normal, true)]),
            c("mailfabrica-engine", &[("mailfabrica-sync", Normal, true), ("mailfabrica-store", Normal, true)]),
            c("mailfabrica-ui-egui", &[("mailfabrica-engine", Normal, true), ("egui", Normal, false), ("rfd", Normal, false)]),
            c("mailfabrica-mcp", &[("mailfabrica-engine", Normal, true)]),
            c("mailfabrica", &[("mailfabrica-ui-egui", Normal, true), ("mailfabrica-mcp", Normal, true), ("eframe", Normal, false)]),
            c("mailfabrica-cli", &[("mailfabrica-mcp", Normal, true), ("mailfabrica-engine", Normal, true)]),
            c("xtask", &[("serde_json", Normal, false)]),
        ];
        assert!(check(&g).is_empty(), "{:?}", check(&g));
    }

    #[test]
    fn upward_dependency_flagged() {
        let v = check(&[c("mailfabrica-proto", &[("mailfabrica-engine", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 1, to: 5, .. }]), "{v:?}");
        let v = check(&[c("mailfabrica-store", &[("mailfabrica-sync", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 2, to: 3, .. }]), "{v:?}");
    }

    #[test]
    fn mime_and_proto_must_not_depend_on_each_other() {
        let v = check(&[c("mailfabrica-mime", &[("mailfabrica-proto", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 1, to: 1, .. }]), "{v:?}");
        let v = check(&[c("mailfabrica-proto", &[("mailfabrica-mime", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 1, to: 1, .. }]), "{v:?}");
        // dev 依赖也算：测试代码同样不该越层。
        assert!(!check(&[c("mailfabrica-proto", &[("mailfabrica-mime", Dev, true)])]).is_empty());
    }

    #[test]
    fn sideways_dependency_flagged() {
        let v = check(&[c("mailfabrica-ui-egui", &[("mailfabrica-mcp", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 6, to: 6, .. }]), "{v:?}");
        let v = check(&[c("mailfabrica-mcp", &[("mailfabrica-ui-egui", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 6, to: 6, .. }]), "{v:?}");
    }

    #[test]
    fn core_depends_on_no_workspace_crate() {
        let v = check(&[c("mailfabrica-core", &[("mailfabrica-mime", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 0, to: 1, .. }]), "{v:?}");
    }

    #[test]
    fn self_dev_dependency_ignored() {
        assert!(check(&[c("mailfabrica-store", &[("mailfabrica-store", Dev, true)])]).is_empty());
    }

    #[test]
    fn upward_dev_dependency_flagged() {
        let v = check(&[c("mailfabrica-core", &[("mailfabrica-store", Dev, true)])]);
        assert!(matches!(v[..], [Violation::Upward { kind: Dev, .. }]), "{v:?}");
    }

    #[test]
    fn ui_crates_below_l6_flagged() {
        for dep in ["egui", "eframe", "winit", "egui_kittest", "egui_extras", "rfd", "wgpu", "wgpu-core"] {
            let v = check(&[c("mailfabrica-engine", &[(dep, Normal, false)])]);
            assert!(matches!(v[..], [Violation::UiBelowL6 { layer: 5, .. }]), "{dep}: {v:?}");
        }
        for krate in ["mailfabrica-core", "mailfabrica-mime", "mailfabrica-proto", "mailfabrica-store", "mailfabrica-sync"] {
            assert!(!check(&[c(krate, &[("egui", Normal, false)])]).is_empty(), "{krate}");
        }
        assert!(check(&[c("mailfabrica-engine", &[("serde", Normal, false)])]).is_empty());
        // L6 及以上可以用。
        assert!(check(&[c("mailfabrica-mcp", &[("winit", Normal, false)])]).is_empty());
        assert!(check(&[c("mailfabrica-ui-egui", &[("egui", Normal, false), ("rfd", Normal, false)])]).is_empty());
    }

    #[test]
    fn unregistered_crate_is_error() {
        let v = check(&[c("mailfabrica-mystery", &[])]);
        assert!(matches!(&v[..], [Violation::Unregistered { krate }] if krate == "mailfabrica-mystery"));
        assert!(v[0].to_string().contains("register"));
    }

    #[test]
    fn apps_and_xtask_exempt() {
        for app in ["mailfabrica", "mailfabrica-cli", "xtask"] {
            assert!(check(&[c(app, &[("egui", Normal, false), ("mailfabrica-ui-egui", Normal, true)])]).is_empty(), "{app}");
        }
    }

    #[test]
    fn app_name_classification() {
        assert_eq!(classify("mailfabrica"), Some(Class::Layer(7)));
        assert_eq!(classify("mailfabrica-cli"), Some(Class::Layer(7)));
        assert_eq!(classify("xtask"), Some(Class::Exempt));
        assert_eq!(classify("mailfabrica-core"), Some(Class::Layer(0)));
        assert_eq!(classify("nope"), None);
    }

    #[test]
    fn metadata_parsing() {
        let meta: Value = serde_json::from_str(
            r#"{"packages":[
                {"name":"mailfabrica-store","dependencies":[
                    {"name":"mailfabrica-core","kind":null,"path":"/x/crates/core"},
                    {"name":"serde","kind":null},
                    {"name":"tempfile","kind":"dev"}]},
                {"name":"mailfabrica-core","dependencies":[]}
            ]}"#,
        )
        .unwrap();
        let g = from_metadata(&meta).unwrap();
        assert_eq!(g.len(), 2);
        let store = g.iter().find(|c| c.name == "mailfabrica-store").unwrap();
        assert!(store.deps[0].workspace && !store.deps[1].workspace);
        assert_eq!(store.deps[2].kind, Dev);
        assert!(check(&g).is_empty());
    }
}
