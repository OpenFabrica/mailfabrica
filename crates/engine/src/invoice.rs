//! 发票邮件：识别（`Scope::Invoices` 虚拟文件夹）和一键导出（附件存到目录 + 生成清单 CSV）。
//!
//! 识别只看列表元数据（主题、发件人），不用下载正文。导出时正文不在本地的先下载，
//! 全部到齐（或失败）后写清单。下载走正常的 `FetchBody` 任务，结果在 `pump()` 里处理。
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use mailfabrica_core::{Folder, FolderRole, MessageMeta, MessageView, MsgKey, date};
use serde::Serialize;

use crate::{BodyState, EngineError, Result, Session, UiRequest, view};

/// 主题/发件人里的发票关键词（小写比较）。
const KEYWORDS: &[&str] = &["发票", "invoice", "fapiao", "电子票据", "开票", "票据"];
/// 发票文件的扩展名。
const INVOICE_EXT: &[&str] = &["pdf", "ofd", "xml", "zip"];
/// 正文里的发票下载链接最多记几个。
const MAX_LINKS: usize = 3;
/// 一次最多导出多少封（防止误操作把整个邮箱都下载一遍）。
const MAX_EXPORT: usize = 2000;

/// 看起来是发票邮件：主题、发件人名字或邮箱里有发票关键词。
pub fn is_invoice(meta: &MessageMeta) -> bool {
    let hay = format!("{} {} {}", meta.subject, meta.from.name.as_deref().unwrap_or_default(), meta.from.email).to_lowercase();
    KEYWORDS.iter().any(|k| hay.contains(k))
}

/// 发票日期范围：(起始 Unix 秒, 结束 Unix 秒，不含)，`None` 表示这一头不限。
pub type InvoiceRange = (Option<i64>, Option<i64>);

/// 解析发票日期范围：`YYYY-MM-DD`（本地时区），两头都含当天，空串表示不限。
/// 错误是给用户看的英文短句（界面按它翻译）。
pub fn invoice_range(since: &str, until: &str) -> std::result::Result<InvoiceRange, &'static str> {
    let parse = |s: &str, f: fn(&str) -> Option<i64>| {
        if s.trim().is_empty() { Ok(None) } else { f(s).map(Some).ok_or("Dates must look like 2026-10-01") }
    };
    let from = parse(since, date::day_start)?;
    let to = parse(until, date::day_end)?;
    if let (Some(a), Some(b)) = (from, to)
        && a >= b
    {
        return Err("The start date is after the end date");
    }
    Ok((from, to))
}

/// 发票范围包含的文件夹：正常邮件文件夹，但不含已发送和草稿（自己转发出去的不算收到的发票）。
pub(crate) fn invoice_folder(f: &Folder) -> bool {
    view::counts_as_mail(f) && !matches!(f.role, FolderRole::Sent | FolderRole::Drafts)
}

/// 导出进度和结果（`invoice.status`、界面状态栏用）。
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceExport {
    pub busy: bool,
    /// 导出目录。
    pub dir: String,
    /// 要处理的邮件数。
    pub total: usize,
    /// 已处理（成功或失败）的邮件数。
    pub done: usize,
    /// 存下的发票文件数。
    pub files: usize,
    /// 处理失败的邮件：`主题: 原因`。
    pub failed: Vec<String>,
    /// 清单 CSV 的路径（完成后才有）。
    pub csv: Option<String>,
    /// 整体失败的原因（例如写不了清单）。
    pub error: Option<String>,
}

/// 清单里的一行。
#[derive(Clone, Debug, Default)]
struct Row {
    date: i64,
    name: String,
    email: String,
    subject: String,
    amount: String,
    files: Vec<String>,
    links: Vec<String>,
    key: String,
}

/// 会话私有的导出状态：公开的进度 + 还在等正文的邮件 + 已收集的行。
#[derive(Default)]
pub(crate) struct Exporter {
    pub(crate) state: InvoiceExport,
    waiting: HashSet<MsgKey>,
    rows: Vec<Row>,
    dir: PathBuf,
}

impl Session {
    /// 全部账户里的发票邮件，最新的在前。
    pub fn invoice_messages(&mut self) -> Vec<MessageMeta> {
        self.invoice_messages_in((None, None))
    }

    /// 发信日期落在 `range`（见 [`invoice_range`]）里的发票邮件，最新的在前。
    pub fn invoice_messages_in(&mut self, range: InvoiceRange) -> Vec<MessageMeta> {
        let within = |m: &MessageMeta| range.0.is_none_or(|a| m.date >= a) && range.1.is_none_or(|b| m.date < b);
        let mut out: Vec<MessageMeta> = Vec::new();
        for a in self.account_ids() {
            for f in self.store.folders(&a).into_iter().filter(invoice_folder) {
                out.extend(self.store.messages(&a, &f.path).unwrap_or_default().into_iter().filter(|m| is_invoice(m) && within(m)));
            }
        }
        out.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.key.cmp(&a.key)));
        out
    }

    /// 导出进度。
    pub fn invoice_export(&self) -> &InvoiceExport {
        &self.invoices.state
    }

    /// 开始导出：本地有原文的马上处理，没有的提交下载，到齐后写清单。
    pub(crate) fn start_invoice_export(&mut self, dir: PathBuf, keys: Vec<MsgKey>) -> Result<()> {
        if self.invoices.state.busy {
            return Err(EngineError::Other("an invoice export is already running".into()));
        }
        if keys.is_empty() {
            return Err(EngineError::Other("no invoice messages found".into()));
        }
        if keys.len() > MAX_EXPORT {
            return Err(EngineError::Other(format!("too many messages ({}); export at most {MAX_EXPORT} at a time", keys.len())));
        }
        std::fs::create_dir_all(&dir).map_err(|e| EngineError::Other(format!("{}: {e}", dir.display())))?;
        self.invoices = Exporter {
            state: InvoiceExport { busy: true, dir: dir.to_string_lossy().into_owned(), total: keys.len(), ..Default::default() },
            dir,
            ..Default::default()
        };
        for key in keys {
            if self.store.raw(&key).is_some() {
                self.export_one(&key);
                continue;
            }
            self.invoices.waiting.insert(key.clone());
            // 提交下载；提交不了（没有密码等）时 ensure_body 记成 Failed，马上算失败
            if let BodyState::Failed(e) = self.ensure_body(&key) {
                self.invoice_body_failed(&key, &e);
            }
        }
        self.finish_invoice_export_if_done();
        self.revision += 1;
        Ok(())
    }

    /// 正文到了：如果导出在等它，就处理。
    pub(crate) fn invoice_body_arrived(&mut self, key: &MsgKey) {
        if self.invoices.waiting.remove(key) {
            self.export_one(key);
            self.finish_invoice_export_if_done();
        }
    }

    /// 正文下载失败：如果导出在等它，记一条失败。
    pub(crate) fn invoice_body_failed(&mut self, key: &MsgKey, why: &str) {
        if self.invoices.waiting.remove(key) {
            let subject = self.store.meta(key).map(|m| m.subject).unwrap_or_else(|| key.to_string());
            self.invoices.state.failed.push(format!("{subject}: {why}"));
            self.invoices.state.done += 1;
            self.finish_invoice_export_if_done();
        }
    }

    /// 处理一封：存发票附件，记一行清单。
    fn export_one(&mut self, key: &MsgKey) {
        self.invoices.state.done += 1;
        let Some(raw) = self.store.raw(key) else {
            self.invoices.state.failed.push(format!("{key}: the message is not downloaded"));
            return;
        };
        let view = mailfabrica_mime::parse(&raw);
        let meta = self.store.meta(key);
        let from = view.from.first().cloned().or_else(|| meta.as_ref().map(|m| m.from.clone())).unwrap_or_default();
        let date = if view.date > 0 { view.date } else { meta.as_ref().map_or(0, |m| m.date) };
        let mut row = Row {
            date,
            name: from.name.clone().unwrap_or_default(),
            email: from.email.clone(),
            subject: view.subject.clone(),
            amount: find_amount(&format!("{}\n{}", view.subject, view.text)).unwrap_or_default(),
            links: invoice_links(&view.text),
            key: key.to_string(),
            ..Default::default()
        };
        let sender = from.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| from.email.clone());
        let prefix = format!("{}_{}", mailfabrica_core::date::date(date), safe_name(&sender, 30));
        for info in view.attachments.iter().filter(|a| is_invoice_file(&a.name)) {
            let Some(att) = mailfabrica_mime::attachment(&raw, info.index) else { continue };
            let name = format!("{prefix}_{}", safe_name(&att.name, 80));
            match save_unique(&self.invoices.dir, &name, &att.data) {
                Ok(saved) => {
                    row.files.push(saved);
                    self.invoices.state.files += 1;
                }
                Err(e) => self.invoices.state.failed.push(format!("{}: {e}", view.subject)),
            }
        }
        self.invoices.rows.push(row);
    }

    /// 全部处理完：写清单，提示用户。
    fn finish_invoice_export_if_done(&mut self) {
        let st = &self.invoices.state;
        if !st.busy || !self.invoices.waiting.is_empty() || st.done < st.total {
            return;
        }
        self.invoices.rows.sort_by_key(|r| std::cmp::Reverse(r.date));
        let en = crate::i18n::is_en(&self.prefs.language);
        let path = self.invoices.dir.join(if en { "invoices.csv" } else { "发票清单.csv" });
        match std::fs::write(&path, csv(&self.invoices.rows, en)) {
            Ok(()) => self.invoices.state.csv = Some(path.to_string_lossy().into_owned()),
            Err(e) => self.invoices.state.error = Some(format!("{}: {e}", path.display())),
        }
        self.invoices.state.busy = false;
        let st = self.invoices.state.clone();
        let text = match &st.error {
            Some(e) => self.tr("invoiceFailed", &[e]),
            None => self.tr("invoiceDone", &[&st.total, &st.files, &st.dir]),
        };
        self.status.text = text.clone();
        self.ui_requests.push(UiRequest::Toast(text));
        self.revision += 1;
    }
}

/// 附件名像发票文件：扩展名是 pdf/ofd/xml/zip，或名字里有"发票"/invoice。
fn is_invoice_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    let ext = Path::new(&lower).extension().and_then(|e| e.to_str()).unwrap_or_default().to_string();
    INVOICE_EXT.contains(&ext.as_str()) || lower.contains("发票") || lower.contains("invoice")
}

/// 文件名里不能有的字符换成 `_`，限制长度（按字符）；空名用 `invoice`。
fn safe_name(s: &str, max: usize) -> String {
    // 附件名来自邮件，不可信：去掉路径分隔符和控制字符，防止写出目录
    let cleaned: String =
        s.chars().map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    let name = if cleaned.is_empty() { "invoice".to_string() } else { cleaned };
    let count = name.chars().count();
    if count <= max {
        return name;
    }
    // 太长时保留扩展名
    let ext = Path::new(&name).extension().and_then(|e| e.to_str()).filter(|e| e.chars().count() < 10).map(|e| format!(".{e}")).unwrap_or_default();
    let keep = max.saturating_sub(ext.chars().count());
    format!("{}{ext}", name.chars().take(keep).collect::<String>())
}

/// 写文件，重名时加 `(2)`、`(3)`…；同名且内容相同就不再写（重复导出不会多出副本）。返回实际文件名。
fn save_unique(dir: &Path, name: &str, data: &[u8]) -> std::io::Result<String> {
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name).to_string();
    let ext = path.extension().and_then(|e| e.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
    for n in 1..1000 {
        let candidate = if n == 1 { name.to_string() } else { format!("{stem}({n}){ext}") };
        let target = dir.join(&candidate);
        match std::fs::read(&target) {
            Ok(existing) if existing == data => return Ok(candidate),
            Ok(_) => continue,
            Err(_) if !target.exists() => {
                std::fs::write(&target, data)?;
                return Ok(candidate);
            }
            Err(_) => continue,
        }
    }
    Err(std::io::Error::other("too many files with the same name"))
}

/// 从文字里找金额：先找"价税合计/金额/合计/total/amount"后面的数字，再找 ¥/￥ 后面的数字。
pub(crate) fn find_amount(text: &str) -> Option<String> {
    const LABELS: &[&str] = &["价税合计", "开票金额", "发票金额", "合计金额", "金额", "合计", "total", "amount"];
    let lower = text.to_lowercase();
    for label in LABELS.iter().chain(["¥", "￥"].iter()) {
        let mut from = 0;
        while let Some(i) = lower.get(from..).and_then(|s| s.find(label)) {
            let start = from + i + label.len();
            if let Some(n) = number_after(lower.get(start..).unwrap_or_default()) {
                return Some(n);
            }
            from = start;
        }
    }
    None
}

/// 标签后面 20 个字符以内的第一个数字（允许千分位逗号和小数点，比如 `：¥1,234.50`）。
fn number_after(s: &str) -> Option<String> {
    let mut chars = s.chars().peekable();
    let mut skipped = 0;
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            break;
        }
        // 碰到换行或另一句话就放弃，免得取到不相干的数字
        if c == '\n' || c == '。' || skipped >= 20 {
            return None;
        }
        chars.next();
        skipped += 1;
    }
    let mut num = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() || c == '.' || c == ',' {
            num.push(c);
            chars.next();
        } else {
            break;
        }
    }
    let num = num.trim_end_matches(['.', ',']).replace(',', "");
    // 金额要像钱：有小数点或至少两位，排除"第1张"这类
    let ok = !num.is_empty() && num.parse::<f64>().is_ok_and(|v| v > 0.0) && (num.contains('.') || num.len() >= 2);
    ok.then_some(num)
}

/// 正文里像发票下载地址的链接（含 pdf/ofd/download/invoice/fapiao）。
fn invoice_links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '（' | '）' | '(' | ')')) {
        let Some(i) = word.find("http") else { continue };
        let url = word.get(i..).unwrap_or_default().trim_end_matches(['.', ',', '。', '，', ';', '；']);
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            continue;
        }
        let lower = url.to_lowercase();
        if ["pdf", "ofd", "download", "invoice", "fapiao"].iter().any(|k| lower.contains(k)) && !out.iter().any(|u| u == url) {
            out.push(url.to_string());
            if out.len() >= MAX_LINKS {
                break;
            }
        }
    }
    out
}

/// 清单 CSV（UTF-8 带 BOM，Excel 直接打开不乱码）。
fn csv(rows: &[Row], en: bool) -> String {
    let head = if en {
        "Date,Sender,Email,Subject,Amount,Files,Download links,Message"
    } else {
        "日期,发件人,邮箱,主题,金额,发票文件,下载链接,邮件"
    };
    let mut out = format!("\u{feff}{head}\r\n");
    for r in rows {
        let cells = [
            mailfabrica_core::date::date(r.date),
            r.name.clone(),
            r.email.clone(),
            r.subject.clone(),
            r.amount.clone(),
            r.files.join("; "),
            r.links.join(" "),
            r.key.clone(),
        ];
        let line: Vec<String> = cells.iter().map(|c| csv_cell(c)).collect();
        out.push_str(&line.join(","));
        out.push_str("\r\n");
    }
    out
}

/// 一个 CSV 单元格：有逗号、引号、换行时加引号；以 `= + - @` 开头时前面加 `'`，防止表格软件当公式执行。
fn csv_cell(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@']) { format!("'{s}") } else { s.to_string() };
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s }
}

/// 发票邮件的阅读视图里有没有发票附件（界面在阅读窗格里给提示用）。
pub fn invoice_attachments(view: &MessageView) -> usize {
    view.attachments.iter().filter(|a| is_invoice_file(&a.name)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 各种写法的金额。
    #[test]
    fn amounts() {
        assert_eq!(find_amount("价税合计（小写）：¥1,234.50").as_deref(), Some("1234.50"));
        assert_eq!(find_amount("您的发票金额为 88.00 元").as_deref(), Some("88.00"));
        assert_eq!(find_amount("Total: $19.99").as_deref(), Some("19.99"));
        assert_eq!(find_amount("消费 ￥36").as_deref(), Some("36"));
        assert_eq!(find_amount("第1张发票").as_deref(), None);
        assert_eq!(find_amount("金额\n\n\n见附件").as_deref(), None);
    }

    /// 文件名清理：去掉路径和危险字符，保留扩展名。
    #[test]
    fn names() {
        assert_eq!(safe_name("../../etc/passwd", 80), "_.._etc_passwd");
        assert_eq!(safe_name("", 80), "invoice");
        let long = format!("{}.pdf", "发".repeat(100));
        let s = safe_name(&long, 20);
        assert!(s.ends_with(".pdf") && s.chars().count() == 20);
        assert!(is_invoice_file("电子发票.PDF") && is_invoice_file("x.ofd") && !is_invoice_file("logo.png"));
    }

    /// CSV 转义和公式注入防护。
    #[test]
    fn csv_cells() {
        assert_eq!(csv_cell("a,b"), "\"a,b\"");
        assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_cell("=1+1"), "'=1+1");
        assert_eq!(csv_cell("plain"), "plain");
    }

    /// 只收像发票下载的链接，去重、去尾标点。
    #[test]
    fn links() {
        let t = "下载：https://fp.example.com/download?id=1。 首页 https://example.com 再次 https://fp.example.com/download?id=1";
        assert_eq!(invoice_links(t), vec!["https://fp.example.com/download?id=1".to_string()]);
    }

    /// 重名加序号，内容相同不重复写。
    #[test]
    fn unique_files() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(save_unique(dir.path(), "a.pdf", b"1").ok().as_deref(), Some("a.pdf"));
        assert_eq!(save_unique(dir.path(), "a.pdf", b"1").ok().as_deref(), Some("a.pdf"));
        assert_eq!(save_unique(dir.path(), "a.pdf", b"2").ok().as_deref(), Some("a(2).pdf"));
    }
}
