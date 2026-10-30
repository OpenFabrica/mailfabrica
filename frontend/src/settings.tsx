import { t } from "./i18n";
import { useEffect, useState } from "react";
import { Plus, Trash2, History } from "lucide-react";
import { Dialog, IconButton } from "./components";
import { execute } from "./api";
import type { Account, Params } from "./types";
import type { ContactIdentity } from "./contact-timeline";
import type { SessionUI } from "./use-session";
export { AccountDialog } from "./account-dialog";
// Settings 将持久化偏好、AI 配置与更新状态统一展示。
export function Settings({
  ui,
  onClose,
  onAccount,
}: {
  ui: SessionUI;
  onClose: () => void;
  onAccount: (account?: Account) => void;
}) {
  const { state, run } = ui;
  const [tab, setTab] = useState("general");
  const [ai, setAI] = useState({
    provider: "anthropic",
    baseUrl: "",
    model: "",
    apiKey: "",
  });
  const [hasKey, setHasKey] = useState(false);
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    if (tab === "ai")
      void execute<{
        provider: string;
        baseUrl: string;
        model: string;
        hasKey: boolean;
      }>("ai.config")
        .then((c) => {
          setAI((a) => ({ ...a, ...c, apiKey: "" }));
          setHasKey(c.hasKey);
        })
        .catch((e) => ui.setError(String(e)));
  }, [tab]);
  if (!state) return null;
  const pref = (key: string, value: unknown) => {
    void run("app.prefs", { [key]: value });
  };
  return (
    <Dialog
      title={t("设置")}
      onClose={onClose}
      wide
      className="settings-dialog"
    >
      <div className="settings-layout">
        <nav>
          {[
            ["general", t("通用")],
            ["accounts", t("邮箱账户")],
            ["ai", t("AI 助手")],
            ["updates", t("应用更新")],
          ].map(([id, title]) => (
            <button
              className={tab === id ? "active" : ""}
              key={id}
              onClick={() => setTab(id)}
            >
              {title}
            </button>
          ))}
        </nav>
        <div className="settings-content" key={tab}>
          {tab === "general" && (
            <>
              <h3>{t("外观")}</h3>
              <div className="setting-row">
                <span>
                  {state.prefs.language === "en" ? "Theme" : t("主题")}
                </span>
                <select
                  value={String(state.prefs.theme || "system")}
                  onChange={(e) => pref("theme", e.target.value)}
                >
                  <option value="system">{t("跟随系统")}</option>
                  <option value="light">{t("浅色")}</option>
                  <option value="dark">{t("深色")}</option>
                </select>
              </div>
              <div className="setting-row">
                <span>{t("界面语言")}</span>
                <select
                  value={String(state.prefs.language)}
                  onChange={(e) => pref("language", e.target.value)}
                >
                  <option value="zh-CN">{t("简体中文")}</option>
                  <option value="en">English</option>
                </select>
              </div>
              <h3>{t("窗口与后台")}</h3>
              <label className="setting-row">
                {t("关闭窗口后常驻菜单栏 / 系统托盘")}
                <input
                  type="checkbox"
                  checked={Boolean(state.prefs.closeToTray)}
                  onChange={(e) => pref("closeToTray", e.target.checked)}
                />
              </label>
              <h3>{t("阅读与写信")}</h3>
              {[
                ["markReadOnOpen", t("打开邮件时自动标为已读")],
                ["remoteImages", t("自动加载远程图片")],
                ["groupBySubject", t("按主题归类邮件")],
                ["quoteOnReply", t("回复时引用原文")],
                ["composeRich", t("默认使用富文本写信")],
                ["saveSent", t("保存已发送邮件副本")],
              ].map(([key, label]) => (
                <label className="setting-row" key={key}>
                  {label}
                  <input
                    type="checkbox"
                    checked={Boolean(state.prefs[key])}
                    onChange={(e) => pref(key, e.target.checked)}
                  />
                </label>
              ))}
              <div className="setting-row">
                <span>{t("自动收取间隔（分钟，0 为关闭）")}</span>
                <input
                  type="number"
                  min={0}
                  max={1440}
                  value={Number(state.prefs.syncIntervalMinutes)}
                  onChange={(e) =>
                    pref("syncIntervalMinutes", Number(e.target.value))
                  }
                />
              </div>
            </>
          )}
          {tab === "accounts" && (
            <>
              <div className="section-title">
                <h3>{t("邮箱账户")}</h3>
                <button onClick={() => onAccount()}>
                  <Plus size={14} />
                  {t("添加")}
                </button>
              </div>
              {state.accounts.map((a) => (
                <div className="account-setting" key={a.id}>
                  <div>
                    <strong>{a.name || a.email}</strong>
                    <p className="muted">{a.email}</p>
                  </div>
                  <button onClick={() => onAccount(a)}>{t("编辑")}</button>
                  <IconButton
                    label={`删除 ${a.email}`}
                    onClick={() => {
                      if (window.confirm(`删除 ${a.email} 及其本地邮件缓存？`))
                        void run("account.remove", { id: a.id });
                    }}
                  >
                    <Trash2 />
                  </IconButton>
                </div>
              ))}
            </>
          )}
          {tab === "ai" && (
            <form
              className="settings-form"
              onSubmit={(e) => {
                e.preventDefault();
                const p: Params = { ...ai };
                if (!ai.apiKey) delete p.apiKey;
                void run("ai.config", p).then((result) => {
                  if (result !== undefined) setSaved(true);
                });
              }}
            >
              <h3>{t("AI 助手")}</h3>
              <label>
                {t("接口协议")}
                <select
                  value={ai.provider}
                  onChange={(e) =>
                    setAI((a) => ({ ...a, provider: e.target.value }))
                  }
                >
                  <option value="anthropic">Anthropic</option>
                  <option value="openai">{t("OpenAI 兼容")}</option>
                </select>
              </label>
              <label>
                {t("接口地址")}
                <input
                  value={ai.baseUrl}
                  onChange={(e) =>
                    setAI((a) => ({ ...a, baseUrl: e.target.value }))
                  }
                  placeholder={
                    ai.provider === "anthropic"
                      ? "https://api.anthropic.com"
                      : "https://example.com/v1"
                  }
                />
              </label>
              <label>
                {t("模型")}
                <input
                  value={ai.model}
                  onChange={(e) =>
                    setAI((a) => ({ ...a, model: e.target.value }))
                  }
                  placeholder={
                    ai.provider === "anthropic"
                      ? "claude-opus-5-5"
                      : t("模型 ID")
                  }
                />
              </label>
              <label>
                {t("API 密钥")}
                <input
                  type="password"
                  autoComplete="new-password"
                  value={ai.apiKey}
                  onChange={(e) =>
                    setAI((a) => ({ ...a, apiKey: e.target.value }))
                  }
                  placeholder={
                    hasKey ? t("已保存，留空保持") : t("密钥保存在系统凭据中")
                  }
                />
              </label>
              <p className="muted">
                {t("点击 AI 操作时，相关邮件内容会发送到你配置的服务。")}
              </p>
              <footer>
                <button className="primary">
                  {saved ? t("已保存") : t("保存配置")}
                </button>
              </footer>
            </form>
          )}
          {tab === "updates" && (
            <>
              <h3>{t("应用更新")}</h3>
              <p>MailFabrica · Go + Wails</p>
              <p className="muted">
                {(
                  {
                    idle: t("尚未检查"),
                    checking: t("正在检查…"),
                    available: t("有可用更新"),
                    current: t("已是最新版本"),
                    downloading: t("正在下载…"),
                    ready: t("安装包已就绪"),
                    failed: t("更新失败"),
                  } as Record<string, string>
                )[state.update.phase] || state.update.phase}
              </p>
              {state.update.release && (
                <>
                  <strong>{state.update.release.version}</strong>
                  <pre>{state.update.release.notes}</pre>
                </>
              )}
              {state.update.error && (
                <p className="inline-error">{state.update.error}</p>
              )}
              <button
                onClick={() => {
                  void run("update.check");
                }}
              >
                {t("检查更新")}
              </button>
              {state.update.release && (
                <button
                  onClick={() => {
                    void run("update.download");
                  }}
                >
                  {t("下载并安装")}
                </button>
              )}
              {state.update.installer && (
                <button
                  onClick={() => {
                    void run("update.open");
                  }}
                >
                  {t("打开安装器")}
                </button>
              )}
            </>
          )}
        </div>
      </div>
    </Dialog>
  );
}
// Contacts 提供可搜索的真实地址簿和新增联系人表单。
export function Contacts({
  ui,
  onClose,
  onWrite,
  onTimeline,
}: {
  ui: SessionUI;
  onClose: () => void;
  onWrite: (email: string) => void;
  onTimeline: (contact: ContactIdentity) => void;
}) {
  const [items, setItems] = useState<
    {
      email: string;
      name: string;
    }[]
  >([]);
  const [query, setQuery] = useState("");
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const load = () =>
    execute<typeof items>("contacts.list", { query })
      .then(setItems)
      .catch((e) => ui.setError(String(e)));
  useEffect(() => {
    void load();
  }, [query]);
  return (
    <Dialog title={t("联系人")} onClose={onClose}>
      <div className="contacts-content">
        <input
          placeholder={t("搜索联系人")}
          aria-label={t("搜索联系人")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <div className="contact-list">
          {items.map((c) => (
            <div key={c.email}>
              <button
                className="contact-person"
                onClick={() => onWrite(c.email)}
              >
                <strong>{c.name || c.email}</strong>
                <small>{c.email}</small>
              </button>
              <IconButton
                label={`${t("查看往来邮件")} ${c.email}`}
                onClick={() => onTimeline(c)}
              >
                <History size={16} />
              </IconButton>
              <IconButton
                label={`删除联系人 ${c.email}`}
                onClick={() => {
                  void ui.run("contacts.remove", { email: c.email }).then(load);
                }}
              >
                <Trash2 />
              </IconButton>
            </div>
          ))}
          {items.length === 0 && <p className="muted">{t("暂无联系人")}</p>}
        </div>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void ui.run("contacts.add", { email, name }).then(() => {
              setEmail("");
              setName("");
              return load();
            });
          }}
        >
          <input
            aria-label={t("联系人姓名")}
            placeholder={t("姓名")}
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
          <input
            aria-label={t("联系人邮箱")}
            type="email"
            placeholder={t("邮箱")}
            required
            value={email}
            onChange={(e) => setEmail(e.target.value)}
          />
          <button>
            <Plus size={14} />
            {t("添加")}
          </button>
        </form>
      </div>
    </Dialog>
  );
}
