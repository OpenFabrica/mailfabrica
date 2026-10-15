//! AI 写信助手命令：设置接口、写信窗口里起草/改写、采用或放弃建议、快速回复起草。
use mailfabrica_ai::Provider;
use mailfabrica_ai::prompt::{self, ComposeAction, ComposeInput, ReplyInput};
use mailfabrica_core::{MessageView, MsgKey};
use serde_json::{Value as Json, json};

use super::{CommandSpec, always, bad, cmd, has_compose, has_focus, key_param, ok, str_param};
use crate::ai::AI_SECRET;
use crate::{BodyState, ComposeMode, EngineError, Result, Session};

/// AI 命令表。
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("ai.config", "AI Assistant Settings", [], None, "{provider?: \"anthropic\"|\"openai\", baseUrl?, model?, apiKey?}", always, config),
        cmd!(
            "ai.compose",
            "AI Write",
            ["Compose"],
            None,
            "{id, action?: \"write\"|\"polish\"|\"formal\"|\"friendly\"|\"shorter\"|\"longer\"|\"translate\", instruction?}",
            has_compose,
            compose
        ),
        cmd!("ai.accept", "Use AI Suggestion", [], None, "{id, mode?: \"replace\"|\"append\"}", has_compose, accept),
        cmd!("ai.dismiss", "Dismiss AI Suggestion", [], None, "{id}", has_compose, dismiss),
        cmd!("ai.quickReply", "AI Quick Reply", ["Message"], None, "{key?, instruction?}", has_focus, quick_reply),
        cmd!("ai.clearReply", "Clear AI Quick Reply", [], None, "{}", always, clear_reply),
        cmd!(query "ai.status", "AI Assistant Status", [], None, "{}", always, status),
    ]
}

/// 设置的当前值（不含密钥本身）。
fn config_json(s: &Session) -> Json {
    let c = s.ai_config();
    json!({
        "provider": match c.provider { Provider::Anthropic => "anthropic", Provider::OpenAi => "openai" },
        "baseUrl": s.prefs.ai_base_url,
        "model": s.prefs.ai_model,
        "effectiveUrl": c.url(),
        "effectiveModel": c.model(),
        "hasKey": s.ai_has_key(),
        "ready": c.validate().is_ok() && (s.ai_has_key() || c.provider == Provider::OpenAi),
    })
}

/// `ai.config`：改接口设置；`apiKey` 为空串时删除密钥。空参数时只返回当前设置。
fn config(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "ai.config";
    if let Some(v) = str_param(p, "provider") {
        let provider = Provider::parse(v).ok_or_else(|| bad(CMD, "`provider` must be \"anthropic\" or \"openai\""))?;
        s.prefs.ai_provider = match provider {
            Provider::Anthropic => "anthropic",
            Provider::OpenAi => "openai",
        }
        .into();
    }
    if let Some(v) = str_param(p, "baseUrl") {
        s.prefs.ai_base_url = v.trim().to_string();
    }
    if let Some(v) = str_param(p, "model") {
        s.prefs.ai_model = v.trim().to_string();
    }
    if let Some(key) = str_param(p, "apiKey") {
        let key = key.trim();
        if key.is_empty() { s.secrets.delete(AI_SECRET)? } else { s.secrets.set(AI_SECRET, key)? }
    }
    if p.as_object().is_some_and(|o| !o.is_empty()) {
        s.save_prefs()?;
    }
    Ok(config_json(s))
}

/// 写信窗口 id 参数。
fn compose_id(cmd: &str, p: &Json) -> Result<u64> {
    p.get("id").and_then(Json::as_u64).ok_or_else(|| bad(cmd, "`id` (compose window) is required"))
}

/// 正文里用户自己写的部分到哪里结束（字节下标）：签名分隔行 `-- `、转发头、回复引用头之前。
/// 改写和采用建议只动这一段，签名和原文引用保持不动。
pub(crate) fn own_end(body: &str, reply: bool) -> usize {
    let mut pos = 0;
    for line in body.split_inclusive('\n') {
        let l = line.trim_end_matches(['\n', '\r']);
        let quote_head = reply && (l.ends_with("写道：") || l.ends_with(" wrote:"));
        if l == "-- " || l.starts_with("---------- 转发的邮件") || l.starts_with("---------- Forwarded message") || quote_head {
            return pos;
        }
        pos += line.len();
    }
    body.len()
}

/// 原邮件的阅读视图：缓存里有就用，否则解析本地原文。
fn source_view(s: &Session, key: &MsgKey) -> Option<std::sync::Arc<MessageView>> {
    match s.bodies.get(key) {
        Some(BodyState::Ready(v)) => Some(v.clone()),
        _ => s.store.raw(key).map(|raw| std::sync::Arc::new(mailfabrica_mime::parse(&raw))),
    }
}

/// 账户的署名：名字，没有名字时用邮箱。
fn sender_name(s: &Session, account: &str) -> String {
    s.account(account).map(|a| if a.name.trim().is_empty() { a.email } else { a.name }).unwrap_or_default()
}

/// `ai.compose`：按动作在后台生成建议正文，结果放到 `Compose::aiSuggestion`。
fn compose(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "ai.compose";
    let id = compose_id(CMD, p)?;
    let action = match str_param(p, "action") {
        Some(a) => ComposeAction::parse(a).ok_or_else(|| bad(CMD, format!("unknown action `{a}`")))?,
        None => ComposeAction::Write,
    };
    let instruction = str_param(p, "instruction").unwrap_or_default().to_string();
    let c = s.compose(id).cloned().ok_or_else(|| bad(CMD, format!("no compose window {id}")))?;
    if c.sending {
        return Err(EngineError::Other("the message is being sent".into()));
    }
    if c.ai_busy {
        return Err(EngineError::Other("the AI assistant is still working on this message".into()));
    }
    let reply = matches!(c.mode, ComposeMode::Reply | ComposeMode::ReplyAll);
    let draft = c.body.get(..own_end(&c.body, reply)).unwrap_or(&c.body).trim().to_string();
    if action.needs_draft() && draft.is_empty() {
        return Err(EngineError::Other("write something first, then ask the AI assistant to rework it".into()));
    }
    if action == ComposeAction::Write && draft.is_empty() && instruction.trim().is_empty() {
        return Err(bad(CMD, "tell the AI assistant what to write (`instruction`)"));
    }
    let original = c.source.as_ref().and_then(|k| source_view(s, k));
    let from = original.as_ref().and_then(|v| v.from.first()).map(ToString::to_string).unwrap_or_default();
    let sender = sender_name(s, &c.account);
    let input = ComposeInput {
        instruction: &instruction,
        subject: &c.subject,
        to: &c.to,
        sender: &sender,
        draft: &draft,
        original: original.as_ref().map(|v| (from.as_str(), v.text.as_str())),
    };
    let request = prompt::compose(action, &input);
    let task = s.start_ai_compose(id, request)?;
    Ok(json!({"task": task, "action": action.name()}))
}

/// `ai.accept`：采用建议。`replace`（默认）替换自己写的部分，`append` 接在后面；签名和引用不动。
fn accept(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "ai.accept";
    let id = compose_id(CMD, p)?;
    let mode = str_param(p, "mode").unwrap_or("replace");
    if !matches!(mode, "replace" | "append") {
        return Err(bad(CMD, "`mode` must be \"replace\" or \"append\""));
    }
    let c = s.compose_mut(CMD, id)?;
    if c.sending {
        return Err(EngineError::Other("the message is being sent".into()));
    }
    let suggestion = c.ai_suggestion.take().ok_or_else(|| EngineError::Other("there is no AI suggestion to use".into()))?;
    let reply = matches!(c.mode, ComposeMode::Reply | ComposeMode::ReplyAll);
    let end = own_end(&c.body, reply);
    let own = c.body.get(..end).unwrap_or_default().trim_end().to_string();
    let tail = c.body.get(end..).unwrap_or_default().to_string();
    let head = match mode {
        "append" if !own.is_empty() => format!("{own}\n\n{suggestion}"),
        _ => suggestion,
    };
    // 签名/引用前保持原来的空行格式（`\n\n-- \n…`）
    let text = if tail.is_empty() { head } else { format!("{head}\n\n{tail}") };
    c.replace_body(&text, None);
    c.dirty = true;
    c.ai_error = None;
    ok()
}

/// `ai.dismiss`：放弃建议，清掉错误。
fn dismiss(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "ai.dismiss";
    let id = compose_id(CMD, p)?;
    let c = s.compose_mut(CMD, id)?;
    c.ai_suggestion = None;
    c.ai_error = None;
    ok()
}

/// `ai.quickReply`：为一封邮件起草简短回复，结果在 `ai.status` 的 `reply` 里。
fn quick_reply(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "ai.quickReply";
    let key = key_param(s, CMD, p)?;
    let meta = s.store.meta(&key).ok_or_else(|| bad(CMD, format!("no such message: {key}")))?;
    let Some(view) = source_view(s, &key) else {
        s.ensure_body(&key);
        return Err(EngineError::Other("the message is still downloading; try again in a moment".into()));
    };
    let from = view.from.first().cloned().unwrap_or(meta.from).to_string();
    let subject = if view.subject.is_empty() { meta.subject } else { view.subject.clone() };
    let me = sender_name(s, &key.account);
    let instruction = str_param(p, "instruction").unwrap_or_default();
    let request = prompt::reply(&ReplyInput { from: &from, subject: &subject, original: &view.text, instruction, me: &me });
    let task = s.start_ai_reply(&key, request)?;
    Ok(json!({"task": task, "key": key.to_string()}))
}

/// `ai.clearReply`：清掉快速回复草稿（发送或放弃后）。
fn clear_reply(s: &mut Session, _: &Json) -> Result<Json> {
    s.clear_ai_reply();
    ok()
}

/// `ai.status`：设置、是否在忙、快速回复草稿。
fn status(s: &mut Session, _: &Json) -> Result<Json> {
    let mut out = config_json(s);
    out["busy"] = json!(s.ai_busy());
    out["reply"] = json!(s.ai_reply());
    Ok(out)
}
