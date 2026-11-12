import { MailBodyLoading } from "./mail-body-loading";
import { t } from "./i18n";
import { MailTranslation } from "./translation";
import { Fragment, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  ChevronDown,
  Download,
  Flag,
  History,
  LoaderCircle,
  MoreHorizontal,
  Reply,
  ReplyAll,
  Send,
  ShieldCheck,
  Sparkles,
  Trash2,
  ArrowUpRight,
} from "lucide-react";
import type { SessionUI } from "./use-session";
import { execute, pickFile } from "./api";
import { IconButton, dateLabel } from "./components";
import { EmailContent, useEmailTheme } from "./email-content";
import type { ContactIdentity } from "./contact-timeline";
import { useDismiss } from "./use-dismiss";
import { ReaderLanding, type LandingActions } from "./reader-landing";
import { keyString } from "./types";
import {
  ConversationSummary,
  ConversationToolbar,
  SummaryResult,
} from "./conversation";
// Reader 呈现主题组、单封阅读及快速回复，所有修改经 run。
export function Reader({
  ui,
  onCompose,
  onContact,
  landing,
}: {
  ui: SessionUI;
  onCompose: (id?: number) => void;
  onContact: (contact: ContactIdentity) => void;
  landing: LandingActions;
}) {
  const { state, run } = ui;
  const [reply, setReply] = useState("");
  const dark = useEmailTheme(state?.prefs.theme);
  const [plain, setPlain] = useState(false);
  useEffect(() => {
    const mode = (event: Event) =>
      setPlain((event as CustomEvent).detail === "plain");
    document.addEventListener("mailfabrica:readerMode", mode);
    return () => document.removeEventListener("mailfabrica:readerMode", mode);
  }, []);
  const [menu, setMenu] = useState(false);
  const menuRoot = useDismiss(menu, () => setMenu(false));
  const lastAI = useRef(0);
  const currentKey = state?.view.focused;
  useEffect(() => {
    setReply("");
    setPlain(false);
    setMenu(false);
  }, [currentKey]);
  useEffect(() => {
    if (
      state?.aiReply?.text &&
      state.aiReply.key === currentKey &&
      state.aiReply.seq !== lastAI.current
    ) {
      setReply(state.aiReply.text);
      lastAI.current = state.aiReply.seq;
    }
  }, [state?.aiReply, currentKey]);
  const body = state?.message?.body;
  if (!state) return null;
  const en = state.prefs.language === "en";
  if (ui.openingMessage)
    return (
      <section
        className="reader"
        aria-label={en ? "Reading pane" : t("阅读窗格")}
      >
        <MailBodyLoading />
      </section>
    );
  const conversation = state.view.group
    ? state.view.rows.find((r) => r.group === state.view.group && r.count > 1)
    : undefined;
  const summary =
    state.aiGroupSummary?.group === conversation?.group
      ? state.aiGroupSummary
      : null;
  if (conversation && !state.view.focused) {
    const row = conversation;
    return (
      <section className="reader">
        <header className="reader-header">
          <h2>{row?.message.subject}</h2>
          <p className="muted">
            {row?.count}{" "}
            {en ? "messages in this conversation" : t("封相关邮件")}
            {" · "}
            {row.unread} {en ? "unread" : t("封未读")}
          </p>
        </header>
        <div className="conversation-overview">
          <ConversationSummary row={row} ui={ui} />
        </div>
        <div className="group-messages">
          {state.view.messages
            .filter((m) => row?.keys.includes(keyString(m.key)))
            .map((m) => (
              <button
                key={keyString(m.key)}
                className={`group-message ${m.flags.seen ? "read" : "unread"}`}
                data-testid={`reader.group.${keyString(m.key)}`}
                onClick={() => {
                  void run("view.select", { key: keyString(m.key) });
                }}
              >
                <div className="group-message-top">
                  <span
                    className={`unread-dot ${m.flags.seen ? "" : "on"}`}
                    aria-hidden="true"
                  />
                  <strong>{m.from.name || m.from.email}</strong>
                  <span className="message-read-state">
                    {t(m.flags.seen ? "已读" : "未读")}
                  </span>
                  <time>{dateLabel(m.date, en)}</time>
                </div>
                <h3>{m.subject}</h3>
                <p>{m.preview}</p>
              </button>
            ))}
        </div>
      </section>
    );
  }
  if (!state.message) return <ReaderLanding ui={ui} actions={landing} />;
  const meta = state.message.meta;
  if (!meta) return null;
  // 垃圾邮件文件夹中的邮件提供「不是垃圾邮件」，一键移回收件箱
  const inJunk = !!state.folders[meta.key.account]?.some(
    (f) => f.path === meta.key.folder && f.role === "junk",
  );
  const create = async (command: string) => {
    const result = await run<{
      id: number;
    }>(command, { key: currentKey });
    if (result) onCompose(result.id);
  };
  const save = async (index: number, name: string) => {
    try {
      const path = await pickFile("save", name);
      if (path)
        await run("message.saveAttachment", { key: currentKey, index, path });
    } catch (e) {
      ui.setError(String(e));
    }
  };
  return (
    <section
      className="reader"
      aria-label={en ? "Reading pane" : t("阅读窗格")}
    >
      <div className="reader-actions">
        <div className="reader-navigate">
          <IconButton
            label={
              conversation
                ? en
                  ? "Previous conversation"
                  : "上一会话"
                : en
                  ? "Previous"
                  : t("上一封")
            }
            onClick={() => {
              void run("view.prev");
            }}
          >
            <ArrowLeft />
          </IconButton>
          <IconButton
            label={
              conversation
                ? en
                  ? "Next conversation"
                  : "下一会话"
                : en
                  ? "Next"
                  : t("下一封")
            }
            onClick={() => {
              void run("view.next");
            }}
          >
            <ArrowRight />
          </IconButton>
        </div>
        <div>
          <IconButton
            label={en ? "Flag" : t("红旗")}
            className={meta.flags.flagged ? "flagged" : ""}
            onClick={() => {
              void run("message.flag");
            }}
          >
            <Flag />
          </IconButton>
          <IconButton
            label={en ? "Reply" : t("回复")}
            onClick={() => {
              void create("compose.reply");
            }}
          >
            <Reply />
          </IconButton>
          <IconButton
            label={en ? "Reply all" : t("全部回复")}
            onClick={() => {
              void create("compose.replyAll");
            }}
          >
            <ReplyAll />
          </IconButton>
          <IconButton
            label={en ? "Forward" : t("转发")}
            onClick={() => {
              void create("compose.forward");
            }}
          >
            <ArrowUpRight />
          </IconButton>
          {inJunk && (
            <IconButton
              label={t("不是垃圾邮件")}
              data-testid="reader.notJunk"
              onClick={() => {
                void run("message.notJunk");
              }}
            >
              <ShieldCheck />
            </IconButton>
          )}
          <IconButton
            label={en ? "Delete" : t("删除")}
            onClick={() => {
              void run("message.delete");
            }}
          >
            <Trash2 />
          </IconButton>
          <div className="relative" ref={menuRoot}>
            <IconButton
              label={en ? "More" : t("更多")}
              data-testid="reader.more"
              aria-expanded={menu}
              aria-haspopup="true"
              onClick={() => setMenu(!menu)}
            >
              <MoreHorizontal />
            </IconButton>
            {menu && (
              <div className="popup-menu right">
                <button
                  onClick={() => {
                    setPlain(!plain);
                    setMenu(false);
                  }}
                >
                  {plain ? t("显示 HTML") : t("显示纯文本")}
                </button>
                <button
                  onClick={() => {
                    void run("message.toggleRead");
                    setMenu(false);
                  }}
                >
                  {meta.flags.seen ? t("标为未读") : t("标为已读")}
                </button>
                <button
                  onClick={() => {
                    void run(inJunk ? "message.notJunk" : "message.junk");
                    setMenu(false);
                  }}
                >
                  {inJunk ? t("不是垃圾邮件") : t("标为垃圾邮件")}
                </button>
                <label>
                  {t("移动到")}
                  <select
                    defaultValue=""
                    onChange={(e) => {
                      if (e.target.value)
                        void run("message.move", { folder: e.target.value });
                      setMenu(false);
                    }}
                  >
                    <option value="">{t("选择文件夹")}</option>
                    {state.folders[meta.key.account]
                      ?.filter((f) => f.selectable)
                      .map((f) => (
                        <option key={f.path}>{f.path}</option>
                      ))}
                  </select>
                </label>
                {(meta.flags.draft ||
                  state.folders[meta.key.account]?.some(
                    (f) => f.path === meta.key.folder && f.role === "drafts",
                  )) && (
                  <button
                    onClick={() => {
                      void create("compose.editDraft");
                      setMenu(false);
                    }}
                  >
                    {t("继续编辑草稿")}
                  </button>
                )}
              </div>
            )}
          </div>
        </div>
      </div>
      {conversation && <ConversationToolbar row={conversation} ui={ui} />}
      {summary && (
        <SummaryResult
          summary={summary}
          changed={
            summary.keys.length !== conversation?.keys.length ||
            summary.keys.some((k) => !conversation?.keys.includes(k))
          }
          en={en}
        />
      )}
      <header className="reader-header">
        <h2>{meta.subject || (en ? "(No subject)" : t("（无主题）"))}</h2>
        <div className="message-address">
          <div className="avatar">
            {(meta.from.name || meta.from.email).slice(0, 1).toUpperCase()}
          </div>
          <div className="message-contact">
            <div className="sender-heading">
              <button
                className="contact-link sender-link"
                title={t("查看往来邮件")}
                onClick={() => onContact(meta.from)}
              >
                {meta.from.name || meta.from.email}
              </button>
              <span className="muted address-email">
                &lt;{meta.from.email}&gt;
              </span>
              <button
                type="button"
                className="reader-history"
                data-testid="reader.timeline"
                onClick={() => onContact(meta.from)}
              >
                <History size={13} aria-hidden="true" />
                {t("往来时间线")}
              </button>
            </div>
            <p>
              {en ? "To" : t("发给")}{" "}
              {(body?.to || meta.to || []).map((address, index) => (
                <Fragment key={`${address.email}-${index}`}>
                  {index > 0 && "、"}
                  <button
                    className="contact-link"
                    title={t("查看往来邮件")}
                    onClick={() => onContact(address)}
                  >
                    {address.name || address.email}
                  </button>
                </Fragment>
              ))}
              <ChevronDown size={11} />
            </p>
          </div>
          <time>
            {new Date(meta.date * 1000).toLocaleString(en ? "en" : "zh-CN", {
              dateStyle: "medium",
              timeStyle: "short",
            })}
          </time>
        </div>
      </header>
      <div className="reader-content">
        {state.message.state === "loading" && <MailBodyLoading />}
        {state.message.state === "failed" && (
          <div className="inline-error">
            {state.message.error}
            <button
              onClick={() => {
                void run("message.open", { key: currentKey });
              }}
            >
              {t("重试")}
            </button>
          </div>
        )}
        {body && currentKey && (
          <MailTranslation key={currentKey} ui={ui} messageKey={currentKey} />
        )}
        {body && (
          <EmailContent
            key={`body:${currentKey}`}
            showLoading
            title={en ? "Email content" : t("邮件正文")}
            html={plain ? null : body.html}
            text={body.text}
            remoteImages={Boolean(state.prefs.remoteImages)}
            dark={dark}
            onLinkError={ui.setError}
          />
        )}
        <div className="attachments">
          {body?.attachments
            .filter((a) => !a.inline)
            .map((a) => (
              <button
                className="attachment"
                key={a.index}
                onClick={() => {
                  void save(a.index, a.name);
                }}
              >
                <span className="file-icon">
                  {a.name.split(".").at(-1)?.slice(0, 3).toUpperCase()}
                </span>
                <span>
                  <strong>{a.name}</strong>
                  <small>{Math.max(1, Math.round(a.size / 1024))} KB</small>
                </span>
                <Download size={14} />
              </button>
            ))}
        </div>
      </div>
      <form
        className="quick-reply"
        onSubmit={(e) => {
          e.preventDefault();
          void (async () => {
            const result = await run<{
              id: number;
            }>("compose.reply", {
              key: currentKey,
              background: true,
            });
            if (result) {
              await execute("compose.set", { id: result.id, body: reply });
              const sent = await run("compose.send", { id: result.id });
              if (sent !== undefined) setReply("");
            }
          })();
        }}
      >
        <textarea
          aria-label={en ? "Quick reply" : t("快速回复")}
          placeholder={
            en
              ? `Reply to ${meta.from.name || meta.from.email}…`
              : `回复 ${meta.from.name || meta.from.email}…`
          }
          value={reply}
          onChange={(e) => setReply(e.target.value)}
          rows={reply.includes("\n") ? 3 : 1}
        />
        <div>
          <span className="muted">
            {en
              ? "A quick thought, thoughtfully sent."
              : t("简短回复，也能认真表达。")}
          </span>
          <button
            type="button"
            className="text-button accent"
            disabled={state.aiReply?.busy}
            onClick={() => {
              void run("ai.quickReply", { key: currentKey });
            }}
          >
            <Sparkles size={14} />
            {en ? "AI draft" : t("AI 起草")}
          </button>
          <button type="submit" className="primary" disabled={!reply.trim()}>
            <Send size={13} />
            {en ? "Reply" : t("回复")}
          </button>
        </div>
      </form>
    </section>
  );
}
