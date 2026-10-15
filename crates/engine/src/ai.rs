//! AI 写信助手的会话状态：接口设置、后台任务、写信窗口里的建议、快速回复草稿。
//!
//! 生成结果不直接覆盖用户正文：写信窗口里先放进 `Compose::ai_suggestion`，用户用 `ai.accept` 采用；
//! 快速回复的草稿放在 [`AiReply`]，界面拿去填快速回复框。
use std::collections::HashMap;
use std::sync::Arc;

use mailfabrica_ai::{Client, Config, Provider, Request, Worker};
use mailfabrica_core::MsgKey;
use serde::Serialize;

use crate::{EngineError, Result, Session};

/// 密码存储里放 AI 接口密钥的条目名（账户 id 不会含 `:`，不会撞名）。
pub(crate) const AI_SECRET: &str = "ai:api-key";

/// 快速回复的 AI 草稿（`ai.status` 和界面用）。
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiReply {
    /// 为哪封邮件起草（字符串形式 `账户/uid/文件夹`）。
    pub key: String,
    /// 每次请求加一，界面据此判断是不是新结果。
    pub seq: u64,
    pub busy: bool,
    pub text: Option<String>,
    pub error: Option<String>,
}

/// 一个 AI 任务的结果要送到哪里。
#[derive(Clone, Debug)]
enum Target {
    Compose(u64),
    Reply(u64),
}

/// 会话私有的 AI 运行状态。
pub(crate) struct Ai {
    worker: Worker,
    next: u64,
    tasks: HashMap<u64, Target>,
    reply: Option<AiReply>,
}

impl Ai {
    /// 不联网；有请求时才起线程。
    pub(crate) fn new(client: Arc<dyn Client>) -> Self {
        Ai { worker: Worker::new(client), next: 0, tasks: HashMap::new(), reply: None }
    }
}

impl Session {
    /// 测试用：换掉 AI 客户端（不联网）。
    pub fn set_ai_client(&mut self, client: Arc<dyn Client>) {
        self.ai = Ai::new(client);
    }

    /// 当前偏好对应的接口设置。
    pub(crate) fn ai_config(&self) -> Config {
        Config {
            provider: Provider::parse(&self.prefs.ai_provider).unwrap_or_default(),
            base_url: self.prefs.ai_base_url.clone(),
            model: self.prefs.ai_model.clone(),
        }
    }

    /// 是否存了 AI 接口密钥。
    pub fn ai_has_key(&self) -> bool {
        self.secrets.get(AI_SECRET).is_some_and(|k| !k.is_empty())
    }

    /// 有 AI 任务在跑。
    pub fn ai_busy(&self) -> bool {
        !self.ai.tasks.is_empty()
    }

    /// 快速回复的 AI 草稿（没请求过时为 `None`）。
    pub fn ai_reply(&self) -> Option<&AiReply> {
        self.ai.reply.as_ref()
    }

    /// 校验设置、取密钥，然后在后台开始生成。返回任务号。
    fn start_ai(&mut self, target: Target, request: Request) -> Result<u64> {
        let config = self.ai_config();
        config.validate().map_err(EngineError::Other)?;
        let key = self.secrets.get(AI_SECRET).unwrap_or_default();
        // Anthropic 一定要密钥；OpenAI 兼容的本地模型（Ollama）可以不要
        if key.is_empty() && config.provider == Provider::Anthropic {
            return Err(EngineError::Other("Set the AI API key in Settings first".into()));
        }
        self.ai.next = self.ai.next.wrapping_add(1);
        let id = self.ai.next;
        self.ai.worker.start(id, config, key, request).map_err(EngineError::Other)?;
        self.ai.tasks.insert(id, target);
        self.revision += 1;
        Ok(id)
    }

    /// 写信窗口：开始生成建议正文。
    pub(crate) fn start_ai_compose(&mut self, compose: u64, request: Request) -> Result<u64> {
        let id = self.start_ai(Target::Compose(compose), request)?;
        if let Some(c) = self.composes.iter_mut().find(|c| c.id == compose) {
            c.ai_busy = true;
            c.ai_error = None;
        }
        Ok(id)
    }

    /// 快速回复：开始起草；新请求顶替旧的（旧结果到了也丢掉）。
    pub(crate) fn start_ai_reply(&mut self, key: &MsgKey, request: Request) -> Result<u64> {
        let seq = self.ai.reply.as_ref().map_or(1, |r| r.seq.wrapping_add(1));
        self.ai.tasks.retain(|_, t| !matches!(t, Target::Reply(_)));
        let id = self.start_ai(Target::Reply(seq), request)?;
        self.ai.reply = Some(AiReply { key: key.to_string(), seq, busy: true, text: None, error: None });
        Ok(id)
    }

    /// 清掉快速回复草稿。
    pub(crate) fn clear_ai_reply(&mut self) {
        self.ai.tasks.retain(|_, t| !matches!(t, Target::Reply(_)));
        self.ai.reply = None;
    }

    /// 收后台结果，放到对应的写信窗口或快速回复草稿里。有变化时返回 true。
    pub(crate) fn pump_ai(&mut self) -> bool {
        let results = self.ai.worker.poll();
        self.apply_ai(results)
    }

    /// 等 AI 任务完成（最多 `timeout`），CLI 和测试用。
    pub(crate) fn wait_ai(&mut self, timeout: std::time::Duration) {
        if self.ai_busy() {
            let results = self.ai.worker.wait(timeout);
            self.apply_ai(results);
        }
    }

    /// 把结果按任务号送到目标；目标已经不在（窗口关了、被新请求顶替）就丢掉。
    fn apply_ai(&mut self, results: Vec<(u64, std::result::Result<String, String>)>) -> bool {
        let mut changed = false;
        for (id, result) in results {
            let Some(target) = self.ai.tasks.remove(&id) else { continue };
            changed = true;
            match target {
                Target::Compose(cid) => {
                    if let Some(c) = self.composes.iter_mut().find(|c| c.id == cid) {
                        c.ai_busy = false;
                        match result {
                            Ok(text) => {
                                c.ai_suggestion = Some(text);
                                c.ai_error = None;
                            }
                            Err(e) => c.ai_error = Some(e),
                        }
                    }
                }
                Target::Reply(seq) => {
                    if let Some(r) = self.ai.reply.as_mut().filter(|r| r.seq == seq) {
                        r.busy = false;
                        match result {
                            Ok(text) => r.text = Some(text),
                            Err(e) => r.error = Some(e),
                        }
                    }
                }
            }
        }
        if changed {
            self.revision += 1;
        }
        changed
    }
}
