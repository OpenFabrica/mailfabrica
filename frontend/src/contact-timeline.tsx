import { Fragment, useEffect, useRef, useState } from "react";
import {
  ArrowDownLeft,
  ArrowUpRight,
  Mail,
  Paperclip,
  RefreshCw,
  Users,
  LoaderCircle,
} from "lucide-react";
import { execute } from "./api";
import { Dialog, IconButton, folderName } from "./components";
import { ProviderIcon } from "./provider-icon";
import { t } from "./i18n";
import { keyString, type Message } from "./types";
import type { SessionUI } from "./use-session";

// ContactIdentity 让地址簿与阅读头使用同一联系人入口，不要求先保存到地址簿。
export interface ContactIdentity {
  email: string;
  name: string;
}
// TimelineResult 对应只读联系人往来命令及跨账户副本位置。
interface TimelineResult {
  total: number;
  received: number;
  sent: number;
  related: number;
  hasMore: boolean;
  items: { message: Message; direction: string; locations: Message["key"][] }[];
}

// ContactTimeline 用日期轴呈现本地收发历史，通过现有视图命令打开原邮件。
export function ContactTimeline({
  ui,
  contact,
  initialAccount = "",
  onClose,
  onWrite,
  onOpen,
}: {
  ui: SessionUI;
  contact: ContactIdentity;
  initialAccount?: string;
  onClose: () => void;
  onWrite: (email: string) => void;
  onOpen: (message: Message) => Promise<void>;
}) {
  const [account, setAccount] = useState(initialAccount);
  const [result, setResult] = useState<TimelineResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [refresh, setRefresh] = useState(0);
  const [opening, setOpening] = useState("");
  const generation = useRef(0);
  const pageOffset = useRef(0);
  const en = ui.state?.prefs.language === "en";
  useEffect(() => {
    const current = ++generation.current;
    pageOffset.current = 0;
    setResult(null);
    setLoading(true);
    setError("");
    void execute<TimelineResult>("contacts.timeline", {
      email: contact.email,
      account,
      limit: 50,
    })
      .then((next) => {
        if (generation.current === current) {
          pageOffset.current = next.items.length;
          setResult(next);
        }
      })
      .catch((e) => {
        if (generation.current === current) setError(String(e));
      })
      .finally(() => {
        if (generation.current === current) setLoading(false);
      });
    return () => {
      generation.current++;
    };
  }, [contact.email, account, refresh]);
  // 分页只追加当前联系人和账户的结果，切换筛选后丢弃旧请求。
  const more = async () => {
    if (!result || loading) return;
    const current = generation.current;
    setLoading(true);
    setError("");
    try {
      const next = await execute<TimelineResult>("contacts.timeline", {
        email: contact.email,
        account,
        offset: pageOffset.current,
        limit: 50,
      });
      if (generation.current !== current) return;
      pageOffset.current += next.items.length;
      setResult((previous) => {
        if (!previous) return next;
        const seen = new Set(
          previous.items.map(
            (item) => item.message.messageId || keyString(item.message.key),
          ),
        );
        return {
          ...next,
          items: [
            ...previous.items,
            ...next.items.filter(
              (item) =>
                !seen.has(
                  item.message.messageId || keyString(item.message.key),
                ),
            ),
          ],
        };
      });
    } catch (e) {
      if (generation.current === current) setError(String(e));
    } finally {
      if (generation.current === current) setLoading(false);
    }
  };
  return (
    <Dialog
      title={t("往来邮件时间线")}
      onClose={onClose}
      className="contact-timeline-dialog"
      wide
    >
      <header className="timeline-person">
        <div className="avatar">
          {(contact.name || contact.email).slice(0, 1).toUpperCase()}
        </div>
        <div>
          <strong>{contact.name || contact.email}</strong>
          {contact.name && contact.name !== contact.email && (
            <span title={contact.email}>{contact.email}</span>
          )}
        </div>
        <button className="primary" onClick={() => onWrite(contact.email)}>
          <Mail size={14} />
          {t("写邮件")}
        </button>
      </header>
      <div className="timeline-tools">
        <select
          aria-label={t("往来账户范围")}
          value={account}
          onChange={(e) => setAccount(e.target.value)}
        >
          <option value="">{t("全部账户")}</option>
          {ui.state?.accounts.map((a) => (
            <option value={a.id} key={a.id}>
              {a.email}
            </option>
          ))}
        </select>
        <span className="timeline-counts">
          {result && (
            <>
              {t("收件")} {result.received} · {t("发件")} {result.sent}
              {result.related > 0 && (
                <>
                  {" "}
                  · {t("共同收件")} {result.related}
                </>
              )}
            </>
          )}
        </span>
        <IconButton
          label={t("刷新往来邮件")}
          onClick={() => setRefresh((n) => n + 1)}
          disabled={loading}
        >
          <RefreshCw size={15} />
        </IconButton>
      </div>
      <div className="timeline-scroll" aria-busy={loading}>
        {error && (
          <div className="inline-error" role="alert">
            {error}
            <button onClick={() => setRefresh((n) => n + 1)}>
              {t("重试")}
            </button>
          </div>
        )}
        {!loading && !error && result?.total === 0 && (
          <div className="empty">
            <Mail />
            <p>{t("暂无已缓存的往来邮件")}</p>
          </div>
        )}
        <div className="timeline-entries">
          {result?.items.map((item, index) => {
            const m = item.message,
              date = new Date(m.date * 1000);
            const bucket = date.toLocaleDateString(en ? "en" : "zh-CN", {
              year: "numeric",
              month: "long",
            });
            const previous = result.items[index - 1];
            const showMonth =
              !previous ||
              new Date(previous.message.date * 1000).toLocaleDateString(
                en ? "en" : "zh-CN",
                { year: "numeric", month: "long" },
              ) !== bucket;
            const a = ui.state?.accounts.find((a) => a.id === m.key.account);
            const role =
              ui.state?.folders[m.key.account]?.find(
                (f) => f.path === m.key.folder,
              )?.role || "";
            const direction =
              item.direction === "outgoing"
                ? t("已发给")
                : item.direction === "incoming"
                  ? t("收到来自")
                  : t("共同收件");
            const Icon =
              item.direction === "outgoing"
                ? ArrowUpRight
                : item.direction === "incoming"
                  ? ArrowDownLeft
                  : Users;
            const key = keyString(m.key);
            return (
              <Fragment key={key}>
                {showMonth && <h3 className="timeline-month">{bucket}</h3>}
                <div className={`timeline-entry ${item.direction}`}>
                  <span className="timeline-dot">
                    <Icon size={13} />
                  </span>
                  <button
                    className="timeline-mail"
                    disabled={Boolean(opening)}
                    onClick={() => {
                      setOpening(key);
                      setError("");
                      void onOpen(m)
                        .catch((e) => setError(String(e)))
                        .finally(() => setOpening(""));
                    }}
                  >
                    <div className="timeline-meta">
                      <span>
                        {direction}
                        {item.direction !== "related" && (
                          <> {contact.name || contact.email}</>
                        )}
                      </span>
                      <time
                        dateTime={
                          Number.isFinite(date.getTime())
                            ? date.toISOString()
                            : undefined
                        }
                      >
                        {date.toLocaleString(en ? "en" : "zh-CN", {
                          month: "short",
                          day: "numeric",
                          hour: "2-digit",
                          minute: "2-digit",
                        })}
                      </time>
                    </div>
                    <strong>
                      {m.subject || t("（无主题）")}
                      {m.hasAttachments && <Paperclip size={13} />}
                      {opening === key && (
                        <LoaderCircle className="spin" size={13} />
                      )}
                    </strong>
                    {m.preview && <p>{m.preview}</p>}
                    <div
                      className="timeline-source"
                      title={item.locations
                        .map(
                          (k) =>
                            `${ui.state?.accounts.find((a) => a.id === k.account)?.email || k.account} · ${k.folder}`,
                        )
                        .join("\n")}
                    >
                      {a && <ProviderIcon account={a} />}
                      <span>
                        {a?.email || m.key.account} ·{" "}
                        {folderName(m.key.folder, role, en)}
                      </span>
                      {item.locations.length > 1 && (
                        <small>+{item.locations.length - 1}</small>
                      )}
                    </div>
                  </button>
                </div>
              </Fragment>
            );
          })}
        </div>
        {loading && (
          <div className="timeline-loading" role="status">
            <LoaderCircle className="spin" size={15} />
            {t("正在加载…")}
          </div>
        )}
        {result?.hasMore && !loading && (
          <button
            className="timeline-more"
            onClick={() => {
              void more();
            }}
          >
            {t("加载更多")}
          </button>
        )}
      </div>
      <footer className="timeline-footer">
        {result
          ? `${t("已缓存")} ${result.total} ${t("封邮件")}`
          : t("本地已同步邮件")}
      </footer>
    </Dialog>
  );
}
