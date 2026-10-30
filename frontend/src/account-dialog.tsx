import { useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  Check,
  ChevronDown,
  ChevronRight,
  LoaderCircle,
} from "lucide-react";
import { execute } from "./api";
import { Dialog } from "./components";
import { t } from "./i18n";
import { accountProvider, ProviderIcon } from "./provider-icon";
import type { Account, Params, Server } from "./types";
import type { SessionUI } from "./use-session";

// 与旧版保持相同的服务商顺序、两步流程及授权码提示。
const providers = [
  {
    id: "qq",
    name: "QQ 邮箱",
    example: "name@qq.com",
    hint: "在 QQ 邮箱网页版的设置 → 账户中开启 IMAP/SMTP，并生成授权码。",
  },
  {
    id: "exmail",
    name: "腾讯企业邮箱",
    example: "name@company.com",
    hint: "如已开启安全登录，请使用客户端专用密码。",
  },
  {
    id: "netease",
    name: "网易邮箱",
    example: "name@163.com",
    hint: "在网页版设置 → POP3/SMTP/IMAP 中开启 IMAP，并生成客户端授权码。",
  },
  {
    id: "qiye163",
    name: "网易企业邮箱",
    example: "name@company.com",
    hint: "在网页版设置 → 账户与安全 → 客户端设置中开启 IMAP/SMTP，生成并确认客户端授权密码后复制。",
  },
  {
    id: "aliqiye",
    name: "阿里企业邮箱",
    example: "name@company.com",
    hint: "使用企业邮箱密码；如 IMAP 未开启，请联系管理员。",
  },
  {
    id: "gmail",
    name: "Gmail",
    example: "name@gmail.com",
    hint: "使用 Google 账户的应用专用密码。",
  },
  {
    id: "icloud",
    name: "iCloud",
    example: "name@icloud.com",
    hint: "使用在 account.apple.com 创建的 App 专用密码。",
  },
  {
    id: "other",
    name: "其他邮箱",
    example: "name@example.com",
    hint: "服务器按邮箱地址自动填写，也可展开服务器设置手动配置。",
  },
];

// AccountDialog 分离滚动内容与固定操作栏，编辑时从不读回已保存的密码。
export function AccountDialog({
  ui,
  account,
  onClose,
}: {
  ui: SessionUI;
  account?: Account;
  onClose: () => void;
}) {
  const [provider, setProvider] = useState(
    account ? accountProvider(account) : "",
  );
  const [form, setForm] = useState({
    email: account?.email || "",
    name: account?.name || "",
    password: "",
    login: account?.login || "",
    signature: account?.signature || "",
    imapHost: account?.imap.host || "",
    imapPort: account?.imap.port || 993,
    imapSecurity: account?.imap.security || "tls",
    smtpHost: account?.smtp.host || "",
    smtpPort: account?.smtp.port || 465,
    smtpSecurity: account?.smtp.security || "tls",
  });
  const [servers, setServers] = useState(Boolean(account));
  const [working, setWorking] = useState(false);
  const [pendingID, setPendingID] = useState("");
  const [error, setError] = useState("");
  const manualServers = useRef(false);
  const detection = useRef(0);
  const selected = providers.find((p) => p.id === provider) || providers[7];
  const locked = working || Boolean(pendingID);
  useEffect(() => {
    if (!pendingID || !ui.state) return;
    if (ui.state.accounts.some((a) => a.id === pendingID)) onClose();
    else if (!ui.state.status.busy && ui.state.status.errors.length) {
      setPendingID("");
      setServers(true);
      setError(ui.state.status.errors.at(-1)?.[1] || t("连接失败"));
    }
  }, [ui.state, pendingID, onClose]);
  useEffect(
    () => () => {
      detection.current++;
    },
    [],
  );
  const field = (key: keyof typeof form, value: string | number) => {
    if (key === "email") detection.current++;
    setForm((f) => ({ ...f, [key]: value }));
  };
  // 手动修改之后不覆盖服务器；切换服务商或地址时丢弃过期的检测结果。
  const detect = async () => {
    if (
      account ||
      !provider ||
      manualServers.current ||
      !form.email.includes("@")
    )
      return;
    const request = ++detection.current;
    try {
      const result = await execute<{ imap: Server; smtp: Server }>(
        "account.detect",
        { email: form.email.trim(), provider },
      );
      if (request !== detection.current || manualServers.current) return;
      setForm((f) => ({
        ...f,
        imapHost: result.imap.host,
        imapPort: result.imap.port,
        imapSecurity: result.imap.security,
        smtpHost: result.smtp.host,
        smtpPort: result.smtp.port,
        smtpSecurity: result.smtp.security,
      }));
    } catch (e) {
      if (request === detection.current) setError(String(e));
    }
  };
  // 返回列表后改选服务商时，用保留的邮箱重新预填新服务商的服务器。
  useEffect(() => {
    void detect();
  }, [provider]);
  // 新增账户仅在用户改过服务器时传入覆盖值，保留后端多候选自动检测。
  const submit = async () => {
    setWorking(true);
    setError("");
    const p: Params = { ...form, email: form.email.trim(), provider };
    if (account) {
      p.id = account.id;
      delete p.email;
      if (!form.password) delete p.password;
    } else {
      if (!form.login.trim()) delete p.login;
      if (!manualServers.current)
        for (const key of [
          "imapHost",
          "imapPort",
          "imapSecurity",
          "smtpHost",
          "smtpPort",
          "smtpSecurity",
        ])
          delete p[key];
    }
    try {
      const result = await execute<{ id: string }>(
        account ? "account.update" : "account.add",
        p,
      );
      if (account) onClose();
      else setPendingID(result.id);
      await ui.refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setWorking(false);
    }
  };
  return (
    <Dialog
      title={account ? t("编辑账户") : t("添加邮箱账户")}
      onClose={onClose}
      className={`account-dialog ${account ? "account-edit" : "account-add"}`}
    >
      {!provider && !account ? (
        <>
          <div className="provider-picker">
            <h3>{t("选择邮箱类型")}</h3>
            <div className="provider-grid">
              {providers.map((p) => (
                <button
                  type="button"
                  key={p.id}
                  onClick={() => {
                    setProvider(p.id);
                    setError("");
                    manualServers.current = false;
                  }}
                >
                  <ProviderIcon provider={p.id} />
                  <span>
                    <strong>{t(p.name)}</strong>
                    <small>{p.example}</small>
                  </span>
                  <ChevronRight size={16} />
                </button>
              ))}
            </div>
          </div>
          <footer className="account-footer">
            <button onClick={onClose}>{t("取消")}</button>
          </footer>
        </>
      ) : (
        <form
          className="account-form"
          onSubmit={(e) => {
            e.preventDefault();
            void submit();
          }}
        >
          <div className="account-scroll">
            <fieldset className="account-fields" disabled={locked}>
              <div className="account-identity">
                {!account && (
                  <button
                    type="button"
                    className="text-button"
                    onClick={() => {
                      detection.current++;
                      setProvider("");
                      setError("");
                    }}
                  >
                    <ArrowLeft size={15} />
                    {t("返回")}
                  </button>
                )}
                <ProviderIcon account={account} provider={provider} />
                <div>
                  <strong>{account ? account.email : t(selected.name)}</strong>
                  {account && <small>{t("账户设置")}</small>}
                </div>
              </div>
              <section className="account-section">
                <h3>{t("基本信息")}</h3>
                <div className="account-basics">
                  {!account && (
                    <label className="account-full">
                      {t("邮箱地址")}
                      <input
                        type="email"
                        value={form.email}
                        onBlur={() => {
                          void detect();
                        }}
                        onChange={(e) => field("email", e.target.value)}
                        placeholder={selected.example}
                        autoComplete="email"
                        autoFocus
                        required
                      />
                    </label>
                  )}
                  <label>
                    {t("显示名称")}
                    <input
                      value={form.name}
                      onChange={(e) => field("name", e.target.value)}
                      placeholder={t("你的名字")}
                      autoComplete="name"
                      autoFocus={Boolean(account)}
                    />
                  </label>
                  {account && (
                    <label>
                      {t("登录名")}
                      <input
                        value={form.login}
                        onChange={(e) => field("login", e.target.value)}
                        required
                      />
                    </label>
                  )}
                  <label className={account ? "account-full" : ""}>
                    {account
                      ? t("新授权码 / 密码（留空保持）")
                      : t("授权码 / 密码")}
                    <input
                      type="password"
                      autoComplete="new-password"
                      value={form.password}
                      onChange={(e) => field("password", e.target.value)}
                      required={!account}
                    />
                  </label>
                </div>
                {!account && <p className="account-hint">{t(selected.hint)}</p>}
              </section>
              {!account && (
                <button
                  type="button"
                  className="text-button account-server-toggle"
                  aria-expanded={servers}
                  onClick={() => setServers(!servers)}
                >
                  {t("服务器设置")}
                  <ChevronDown
                    size={14}
                    className={servers ? "expanded" : ""}
                  />
                </button>
              )}
              {servers && (
                <section className="account-section">
                  {account && <h3>{t("服务器设置")}</h3>}
                  {!account && (
                    <label className="account-login">
                      {t("登录名")}
                      <input
                        value={form.login}
                        placeholder={form.email || selected.example}
                        onChange={(e) => field("login", e.target.value)}
                      />
                    </label>
                  )}
                  <div className="account-servers">
                    {(["imap", "smtp"] as const).map((prefix) => (
                      <fieldset key={prefix}>
                        <legend>
                          {prefix === "imap"
                            ? t("收件服务器 · IMAP")
                            : t("发件服务器 · SMTP")}
                        </legend>
                        <label>
                          {t("服务器地址")}
                          <input
                            aria-label={`${prefix} 服务器`}
                            value={form[`${prefix}Host`]}
                            placeholder={`${prefix}.example.com`}
                            required
                            onChange={(e) => {
                              manualServers.current = true;
                              field(`${prefix}Host`, e.target.value);
                            }}
                          />
                        </label>
                        <div className="account-server-options">
                          <label>
                            {t("端口")}
                            <input
                              aria-label={`${prefix} 端口`}
                              type="number"
                              min={1}
                              max={65535}
                              value={form[`${prefix}Port`]}
                              required
                              onChange={(e) => {
                                manualServers.current = true;
                                field(`${prefix}Port`, Number(e.target.value));
                              }}
                            />
                          </label>
                          <label>
                            {t("加密方式")}
                            <select
                              aria-label={`${prefix} 加密方式`}
                              value={form[`${prefix}Security`]}
                              onChange={(e) => {
                                manualServers.current = true;
                                field(`${prefix}Security`, e.target.value);
                              }}
                            >
                              <option value="tls">TLS</option>
                              <option value="starttls">STARTTLS</option>
                              <option value="none">{t("本机测试")}</option>
                            </select>
                          </label>
                        </div>
                      </fieldset>
                    ))}
                  </div>
                </section>
              )}
              {account && (
                <section className="account-section">
                  <label>
                    {t("签名")}
                    <textarea
                      value={form.signature}
                      onChange={(e) => field("signature", e.target.value)}
                      rows={3}
                    />
                  </label>
                </section>
              )}
            </fieldset>
          </div>
          <footer className="account-footer">
            <div className="account-feedback" aria-live="polite">
              {error ? (
                <p className="inline-error">{error}</p>
              ) : pendingID ? (
                <p className="muted">{t("正在验证邮箱连接…")}</p>
              ) : null}
            </div>
            <button type="button" onClick={onClose}>
              {t("取消")}
            </button>
            <button className="primary" disabled={locked}>
              {locked ? (
                <LoaderCircle className="spin" size={14} />
              ) : (
                <Check size={14} />
              )}
              {account ? t("保存") : t("连接邮箱")}
            </button>
          </footer>
        </form>
      )}
    </Dialog>
  );
}
