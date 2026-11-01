import { t } from "./i18n";
import { Fragment, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  ChevronDown,
  Download,
  Flag,
  History,
  Inbox,
  LoaderCircle,
  MoreHorizontal,
  Paperclip,
  Reply,
  ReplyAll,
  Send,
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
import { keyString, type Row } from "./types";
import {
  ConversationSummary,
  ConversationToolbar,
  SummaryResult,
} from "./conversation";
// RowItem 保留多选、未读、红旗、主题组与附件的可见反馈。
function RowItem({
  row,
  selected,
  english,
  compact,
  onSelect,
}: {
  row: Row;
  selected: boolean;
  english: boolean;
  compact: boolean;
  onSelect: (extend: boolean) => void;
}) {
  const m = row.message;
  const countLabel = english
    ? `${row.unread} unread / ${row.count} total`
    : `${row.unread} 封未读 / 共 ${row.count} 封`;
  return (
    <button
      className={`mail-row ${selected ? "selected" : ""} ${row.unread ? "unread" : ""}`}
      aria-pressed={selected}
      data-testid={`list.row.${keyString(m.key)}`}
      onClick={(e) => onSelect(e.metaKey || e.ctrlKey)}
    >
      <span className={`unread-dot ${row.unread ? "on" : ""}`} />
      <div className="row-content">
        <div className="row-top">
          <span className="sender">
            <span className="sender-name">{m.from.name || m.from.email}</span>
            {row.count > 1 && (
              <span
                className="count"
                title={countLabel}
                aria-label={countLabel}
              >
                {row.unread}/{row.count}
              </span>
            )}
          </span>
          <time>{dateLabel(m.date, english)}</time>
        </div>
        <div className="row-subject">
          <span>
            {m.subject || (english ? "(No subject)" : t("（无主题）"))}
          </span>
          <span className="row-indicators">
            {row.flagged && <Flag size={12} className="flagged" />}
            {compact && m.hasAttachments && <Paperclip size={12} />}
          </span>
        </div>
        {!compact && (
          <div className="row-preview">
            <span>
              {m.preview ||
                (english ? "Open to download message" : t("打开查看邮件内容"))}
            </span>
            {m.hasAttachments && <Paperclip size={12} />}
          </div>
        )}
      </div>
    </button>
  );
}
// dateGroup 用日历日期分组，保持 Foxmail 式紧凑列表的扫描顺序。
function dateGroup(timestamp: number) {
  const date = new Date(timestamp * 1000),
    now = new Date();
  const day = (d: Date) =>
    Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / 86400000;
  const age = day(now) - day(date);
  if (age === 0) return "今天";
  if (age === 1) return "昨天";
  if (age >= 0 && age <= (now.getDay() + 6) % 7) return "本周";
  if (
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth()
  )
    return "本月";
  return "更早";
}
// MailList 提供分组列表和同一后端搜索范围。
export function MailList({
  ui,
  onCompose,
}: {
  ui: SessionUI;
  onCompose: () => void;
}) {
  const { state, run } = ui;
  const [menu, setMenu] = useState(false);
  const menuRoot = useDismiss(menu, () => setMenu(false));
  if (!state) return null;
  const en = state.prefs.language === "en";
  const titles: Record<string, string> = en
    ? {
        allInboxes: "All inboxes",
        unread: "Unread",
        flagged: "Flagged",
        invoices: "Invoices",
      }
    : {
        allInboxes: t("全部收件箱"),
        unread: t("未读邮件"),
        flagged: t("红旗邮件"),
        invoices: t("发票"),
      };
  const compact = state.prefs.mailListDensity !== "comfortable";
  const scope = state.view.scope;
  const title = titles[scope.kind] || scope.folder || "";
  return (
    <section
      className={`mail-list ${compact ? "compact" : "comfortable"}`}
      aria-label={en ? "Message list" : t("邮件列表")}
    >
      <header className="pane-heading">
        <h1>
          {title}
          <span>{state.view.messages.length}</span>
        </h1>
        <div className="relative" ref={menuRoot}>
          <button
            className="text-button muted"
            aria-expanded={menu}
            data-testid="list.viewMenu"
            onClick={() => setMenu(!menu)}
          >
            {en ? "By date" : t("按时间排序")}
            <ChevronDown size={13} />
          </button>
          {menu && (
            <div className="popup-menu">
              <label>
                <input
                  type="checkbox"
                  checked={Boolean(state.prefs.groupBySubject)}
                  onChange={(e) => {
                    void run("app.prefs", { groupBySubject: e.target.checked });
                    setMenu(false);
                  }}
                />
                {en ? "Group by subject" : t("按主题归类")}
              </label>
              <label>
                <input
                  type="checkbox"
                  checked={compact}
                  onChange={(e) => {
                    void run("app.prefs", {
                      mailListDensity: e.target.checked
                        ? "compact"
                        : "comfortable",
                    });
                    setMenu(false);
                  }}
                />
                {t("紧凑列表")}
              </label>
              <button
                onClick={() => {
                  void run("folder.markAllRead", {});
                  setMenu(false);
                }}
              >
                {en ? "Mark all as read" : t("全部标为已读")}
              </button>
            </div>
          )}
        </div>
      </header>
      <div className="list-scroll">
        {state.view.rows.map((row, index) => {
          const bucket = dateGroup(row.message.date);
          const showDate =
            index === 0 ||
            dateGroup(state.view.rows[index - 1].message.date) !== bucket;
          return (
            <Fragment key={row.keys[0]}>
              {showDate && <div className="date-group">{t(bucket)}</div>}
              <RowItem
                row={row}
                compact={compact}
                english={en}
                selected={row.keys.some((k) => state.view.selected.includes(k))}
                onSelect={(extend) => {
                  void run(
                    row.group && !extend ? "view.selectGroup" : "view.select",
                    row.group && !extend
                      ? { group: row.group }
                      : { key: row.keys[0], extend },
                  );
                }}
              />
            </Fragment>
          );
        })}
        {state.view.rows.length === 0 && (
          <div className="empty">
            <Inbox />
            <p>{en ? "No messages here" : t("这里还没有邮件")}</p>
            <button onClick={onCompose}>
              {en ? "Write a message" : t("写一封邮件")}
            </button>
          </div>
        )}
      </div>
      <footer className="list-footer">
        {state.status.busy ? (
          <>
            <LoaderCircle className="spin" size={12} />
            {en ? "Working…" : t("正在处理…")}
          </>
        ) : en ? (
          `${state.view.messages.length} messages`
        ) : (
          `共 ${state.view.messages.length} 封邮件`
        )}
      </footer>
    </section>
  );
}
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
                    void run("message.junk");
                    setMenu(false);
                  }}
                >
                  {t("标为垃圾邮件")}
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
        {state.message.state === "loading" && (
          <div className="empty">
            <LoaderCircle className="spin" />
            {en ? "Downloading…" : t("正在下载正文…")}
          </div>
        )}
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
        {body && (
          <EmailContent
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
