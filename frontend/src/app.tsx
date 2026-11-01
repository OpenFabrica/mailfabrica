import { t } from "./i18n";
import { lazy, useEffect, useRef, useState } from "react";
import {
  CalendarDays,
  CheckCheck,
  ChevronDown,
  ChevronRight,
  FileText,
  Flag,
  Inbox,
  Mail,
  Plus,
  RefreshCw,
  Search,
  Settings as SettingsIcon,
  SquarePen,
  Users,
  X,
} from "lucide-react";
import { useSession } from "./use-session";
import { nativeBridge } from "./api";
import { IconButton, folderIcon, folderName, Dialog } from "./components";
import { MailList, Reader } from "./mail-view";
const ComposeView = lazy(() =>
  import("./compose-view").then((m) => ({ default: m.ComposeView })),
);
import { AccountDialog, Contacts, Settings } from "./settings";
import { CalendarView } from "./calendar-view";
import { InvoiceDialog } from "./invoice-dialog";
import { FolderDialog } from "./folder-dialog";
import { JunkDialog } from "./junk-dialog";
import { ContactTimeline, type ContactIdentity } from "./contact-timeline";
import { ProviderIcon } from "./provider-icon";
import type { Account, Params } from "./types";
// App 组合桌面壳与各功能区域，不直接持有业务邮件数据。
export function App() {
  const ui = useSession();
  useEffect(() => {
    if (!nativeBridge) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    void import("./control")
      .then(({ attachControl }) => attachControl())
      .then((cancel) => {
        if (disposed) cancel();
        else stop = cancel;
      });
    return () => {
      disposed = true;
      stop?.();
    };
  }, []);
  useEffect(() => {
    const calendar = (event: Event) => {
      const params = (event as CustomEvent).detail;
      setMode(params?.on === false ? "mail" : "calendar");
    };
    document.addEventListener("mailfabrica:calendar", calendar);
    return () => document.removeEventListener("mailfabrica:calendar", calendar);
  }, []);
  const { state, run, error } = ui;
  const [mode, setMode] = useState("mail");
  const [dialog, setDialog] = useState("");
  // 垃圾邮件面板非模态，独立于对话框状态，打开其它对话框时不会被关闭
  const [junkPanel, setJunkPanel] = useState(false);
  const [timelineContact, setTimelineContact] =
    useState<ContactIdentity | null>(null);
  const [timelineBack, setTimelineBack] = useState("");
  const showTimeline = (contact: ContactIdentity, back = "") => {
    setTimelineContact(contact);
    setTimelineBack(back);
    setDialog("timeline");
  };
  const [editing, setEditing] = useState<Account | undefined>();
  const [composeID, setComposeID] = useState<number | null>(null);
  const [query, setQuery] = useState("");
  const [collapsed, setCollapsed] = useState<string[]>([]);
  const collapseLoaded = useRef(false);
  useEffect(() => {
    if (state && !collapseLoaded.current) {
      setCollapsed((state.prefs.collapsed as string[]) || []);
      collapseLoaded.current = true;
    }
  }, [state]);
  useEffect(() => {
    if (collapseLoaded.current) void run("app.prefs", { collapsed });
  }, [collapsed]);
  const searchTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const queryInput = useRef<HTMLInputElement>(null);
  const requestCount = useRef(0);
  const en = state?.prefs.language === "en";
  const standaloneID = Number(
    new URLSearchParams(location.search).get("compose") || 0,
  );
  const openCompose = (id?: number) => {
    if (!nativeBridge && id) setComposeID(id);
  };
  const newCompose = async (params: Params = {}) => {
    const result = await run<{
      id: number;
    }>("compose.new", params);
    if (result) openCompose(result.id);
  };
  useEffect(() => {
    if (!state) return;
    const theme = String(state.prefs.theme || "system");
    document.documentElement.dataset.theme = theme;
    document.documentElement.lang = en ? "en" : "zh-CN";
    document.title = standaloneID
      ? `${en ? "New message" : t("写邮件")} · MailFabrica`
      : "MailFabrica";
  }, [state?.prefs.theme, en, standaloneID]);
  useEffect(() => {
    if (!nativeBridge) return;
    let stop: (() => void) | undefined;
    let disposed = false;
    void import("@wailsio/runtime").then(({ Events }) => {
      if (disposed) return;
      stop = Events.On("mailfabrica:dialog", (event) =>
        setDialog(
          event.data === "accountAdd" || event.data === "accountEdit"
            ? "account"
            : String(event.data),
        ),
      );
    });
    return () => {
      disposed = true;
      stop?.();
    };
  }, []);
  useEffect(() => {
    if (!state || standaloneID || nativeBridge) return;
    const requests = state.requests.slice(requestCount.current);
    requestCount.current = state.requests.length;
    for (const r of requests) {
      if (r.kind === "openCompose" && r.id) setComposeID(r.id);
      if (r.kind === "closeCompose")
        setComposeID((id) => (id === r.id ? null : id));
      if (r.kind === "dialog") setDialog(r.name || "");
    }
  }, [state?.requests, standaloneID]);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      const editable =
        Boolean(dialog) ||
        (e.target as HTMLElement).matches(
          "input,textarea,select,[contenteditable=true]",
        ) ||
        (e.target as HTMLElement).closest(".tiptap,[role=dialog],.popup-menu");
      if ((e.metaKey || e.ctrlKey) && e.key === "n") {
        e.preventDefault();
        void newCompose();
      }
      if ((e.metaKey || e.ctrlKey) && (e.key === "f" || e.key === "k")) {
        e.preventDefault();
        queryInput.current?.focus();
      }
      if (e.key === "F5") {
        e.preventDefault();
        void run("mail.sync");
      }
      if (!editable && mode === "mail") {
        if (e.key === "ArrowDown" || e.key === "ArrowUp") {
          e.preventDefault();
          void run(e.key === "ArrowDown" ? "view.next" : "view.prev");
        }
        if (e.key === "Delete" || e.key === "Backspace") {
          e.preventDefault();
          void run("message.delete");
        }
        if ((e.metaKey || e.ctrlKey) && e.key === "r") {
          e.preventDefault();
          void run<{
            id: number;
          }>("compose.reply").then((c) => c && openCompose(c.id));
        }
      }
      if (e.key === "Escape") {
        setDialog(dialog === "timeline" ? timelineBack : "");
      }
    };
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  }, [run, mode, dialog, timelineBack]);
  useEffect(
    () => () => {
      if (searchTimer.current) clearTimeout(searchTimer.current);
    },
    [],
  );
  if (!state)
    return (
      <main className="loading-screen">
        <div className="brand-mark">
          <Mail />
        </div>
        <strong>MailFabrica</strong>
        <span>{error || t("正在打开邮箱…")}</span>
      </main>
    );
  const compose = state.composes.find(
    (c) => c.id === (standaloneID || composeID),
  );
  if (standaloneID)
    return compose ? (
      <>
        <ComposeView
          key={compose.id}
          compose={compose}
          ui={ui}
          onClose={() => window.close()}
          standalone
        />
        {error && (
          <div className="error-toast" role="alert">
            {error}
          </div>
        )}
      </>
    ) : (
      <main className="loading-screen">
        {en ? "Message closed" : t("写信窗口已关闭")}
      </main>
    );
  const scope = state.view.scope;
  const selectedVirtual = (kind: string) => scope.kind === kind;
  const switchScope = (p: Params) => {
    setMode("mail");
    setQuery("");
    void run("view.scope", p);
  };
  const resize = (e: React.PointerEvent<HTMLDivElement>, index: number) => {
    e.preventDefault();
    const start = e.clientX;
    const initial = Number(
      state.prefs[index === 0 ? "sidebarWidth" : "listWidth"] ||
        (index === 0 ? 220 : 350),
    );
    const target = e.currentTarget;
    target.setPointerCapture(e.pointerId);
    // 限制宽度范围；Retina/触控板的 clientX 可能是小数，引擎只接受整数像素，所以取整
    const width = (event: PointerEvent) =>
      Math.round(
        Math.max(
          index === 0 ? 170 : 280,
          Math.min(index === 0 ? 320 : 560, initial + event.clientX - start),
        ),
      );
    // 宽度变量必须写在 .app 上：它的内联样式会遮住 documentElement 上的同名变量
    const app = target.closest<HTMLElement>(".app");
    const property = index === 0 ? "--sidebar-width" : "--list-width";
    const apply = (value: number) =>
      app?.style.setProperty(property, `${value}px`);
    // 每帧最多重排一次，避免高频 pointermove 让整页反复布局
    let frame = 0;
    let pending = initial;
    const move = (event: PointerEvent) => {
      pending = width(event);
      if (!frame)
        frame = requestAnimationFrame(() => {
          frame = 0;
          apply(pending);
        });
    };
    const up = (event: PointerEvent) => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
      cancelAnimationFrame(frame);
      const value = width(event);
      apply(value);
      // 保存失败时恢复原宽度，避免界面和偏好不一致
      void run("app.prefs", {
        [index === 0 ? "sidebarWidth" : "listWidth"]: value,
      }).then((saved) => saved === undefined && apply(initial));
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
  };
  return (
    <main
      className="app"
      style={
        {
          "--sidebar-width": `${state.prefs.sidebarWidth || 220}px`,
          "--list-width": `${state.prefs.listWidth || 350}px`,
        } as React.CSSProperties
      }
    >
      <header className="app-toolbar">
        <div className="brand">
          <div className="brand-mark">
            <Mail size={15} />
          </div>
          <strong>MailFabrica</strong>
        </div>
        <nav className="top-tabs" aria-label={en ? "Navigation" : t("主导航")}>
          <button
            data-testid="sidebar.mail"
            className={mode === "mail" ? "active" : ""}
            onClick={() => setMode("mail")}
          >
            <Mail size={14} />
            {en ? "Mail" : t("邮件")}
          </button>
          <button
            data-testid="sidebar.calendar"
            className={mode === "calendar" ? "active" : ""}
            onClick={() => setMode("calendar")}
          >
            <CalendarDays size={14} />
            {en ? "Calendar" : t("日历")}
          </button>
          <button onClick={() => setDialog("contacts")}>
            <Users size={14} />
            {en ? "Contacts" : t("联系人")}
          </button>
        </nav>
        <div className="toolbar-right">
          <label className="search-box">
            <Search size={14} />
            <input
              ref={queryInput}
              aria-label={en ? "Search mail" : t("搜索邮件")}
              placeholder={
                en ? "Search messages, people…" : t("搜索邮件、联系人或内容…")
              }
              value={query}
              onChange={(e) => {
                setQuery(e.target.value);
                if (searchTimer.current) clearTimeout(searchTimer.current);
                searchTimer.current = setTimeout(() => {
                  void run("view.search", { query: e.target.value });
                }, 200);
              }}
            />
            {query ? (
              <button
                aria-label={t("清除搜索")}
                onClick={() => {
                  setQuery("");
                  void run("view.search", { query: "" });
                }}
              >
                <X size={12} />
              </button>
            ) : (
              <kbd>⌘ K</kbd>
            )}
          </label>
          <IconButton
            label={en ? "Sync mail" : t("收取邮件")}
            disabled={state.accounts.length === 0}
            onClick={() => {
              void run("mail.sync");
            }}
          >
            <RefreshCw className={state.status.busy ? "spin" : ""} />
          </IconButton>
          <button
            className="primary compose-new"
            data-testid="toolbar.compose"
            onClick={() => {
              if (state.accounts.length === 0) {
                setEditing(undefined);
                setDialog("account");
              } else void newCompose();
            }}
          >
            <SquarePen size={14} />
            {en ? "Compose" : t("写邮件")}
          </button>
          <IconButton
            label={en ? "Settings" : t("设置")}
            onClick={() => setDialog("settings")}
          >
            <SettingsIcon />
          </IconButton>
        </div>
      </header>
      {mode === "mail" ? (
        <div className="mail-workspace">
          <aside className="sidebar">
            <div className="virtual-folders">
              {[
                ["allInboxes", en ? "All inboxes" : t("全部收件箱"), Inbox],
                ["unread", en ? "Unread" : t("未读邮件"), Mail],
                ["flagged", en ? "Flagged" : t("红旗邮件"), Flag],
                ["invoices", en ? "Invoices" : t("发票"), FileText],
              ].map(([kind, label, Icon]) => {
                const K = kind as string;
                const Component = Icon as typeof Mail;
                // 未读角标用引擎的全局计数，不随当前列表范围变化
                const count =
                  K === "unread"
                    ? state.counts.unread
                    : K === "allInboxes"
                      ? Object.values(state.folders)
                          .flat()
                          .filter((f) => f.role === "inbox")
                          .reduce((n, f) => n + f.unread, 0)
                      : null;
                return (
                  <button
                    data-testid={`sidebar.${K}`}
                    key={K}
                    className={selectedVirtual(K) ? "active" : ""}
                    onClick={() => switchScope({ kind: K })}
                  >
                    <Component size={16} />
                    <span>{label as string}</span>
                    {count !== null && count > 0 && <small>{count}</small>}
                  </button>
                );
              })}
            </div>
            <div className="account-folders">
              {state.accounts.map((a) => (
                <div key={a.id} className="account-tree">
                  <div className="account-row">
                    <button
                      className="account-title"
                      title={a.email}
                      aria-label={a.email}
                      aria-expanded={!collapsed.includes(a.id)}
                      onClick={() =>
                        setCollapsed((v) =>
                          v.includes(a.id)
                            ? v.filter((id) => id !== a.id)
                            : [...v, a.id],
                        )
                      }
                    >
                      {collapsed.includes(a.id) ? (
                        <ChevronRight size={12} />
                      ) : (
                        <ChevronDown size={12} />
                      )}
                      <ProviderIcon account={a} />
                      <span className="account-email">{a.email}</span>
                    </button>
                    {/* 该账户有未读时才提供一键已读，悬停显示 */}
                    {state.folders[a.id]?.some((f) => f.unread > 0) && (
                      <IconButton
                        label={en ? "Mark account as read" : t("全部标为已读")}
                        data-testid={`account.markAllRead.${a.id}`}
                        onClick={() => {
                          void run("account.markAllRead", { id: a.id });
                        }}
                      >
                        <CheckCheck />
                      </IconButton>
                    )}
                  </div>
                  {!collapsed.includes(a.id) &&
                    state.folders[a.id]?.map((f) => (
                      <button
                        key={f.path}
                        data-testid={`folder.${a.id}.${f.path}`}
                        disabled={!f.selectable}
                        className={`folder ${scope.kind === "folder" && scope.account === a.id && scope.folder === f.path ? "active" : ""}`}
                        onClick={() =>
                          switchScope({ account: a.id, folder: f.path })
                        }
                      >
                        {folderIcon(f.role)}
                        <span>{folderName(f.path, f.role, en)}</span>
                        {f.unread > 0 && <small>{f.unread}</small>}
                      </button>
                    ))}
                </div>
              ))}
            </div>
            <div className="sidebar-bottom">
              {state.accounts.length > 0 && (
                <button onClick={() => setDialog("folder")}>
                  <Plus size={15} />
                  {en ? "New folder" : "新建文件夹"}
                </button>
              )}
              {scope.kind === "invoices" && (
                <button onClick={() => setDialog("invoice")}>
                  <FileText size={15} />
                  {en ? "Export invoices" : t("导出发票")}
                </button>
              )}
              <button
                onClick={() => {
                  setEditing(undefined);
                  setDialog("account");
                }}
              >
                <Plus size={15} />
                {en ? "Add account" : t("添加邮箱")}
              </button>
              {state.demo && (
                <span className="demo-label">
                  {en ? "Demo mailbox" : t("演示邮箱")}
                </span>
              )}
            </div>
          </aside>
          <div
            className="pane-resizer"
            role="separator"
            aria-label={t("调整侧栏宽度")}
            onPointerDown={(e) => resize(e, 0)}
          />
          <MailList
            ui={ui}
            onCompose={() => {
              void newCompose();
            }}
            onJunk={() => setJunkPanel(true)}
          />
          <div
            className="pane-resizer"
            role="separator"
            aria-label={t("调整列表宽度")}
            onPointerDown={(e) => resize(e, 1)}
          />
          <Reader
            ui={ui}
            onCompose={openCompose}
            onContact={(contact) => showTimeline(contact)}
            landing={{
              onCompose: () => void newCompose(),
              onScope: (kind) => switchScope({ kind }),
              onCalendar: () => setMode("calendar"),
              onContacts: () => setDialog("contacts"),
              onSearch: () => queryInput.current?.focus(),
            }}
          />
        </div>
      ) : (
        <CalendarView
          ui={ui}
          onMail={(key) => {
            setMode("mail");
            void run("view.select", { key });
          }}
        />
      )}
      {error && (
        <div className="error-toast" role="alert">
          <span>{error}</span>
          <IconButton label={t("关闭错误")} onClick={() => ui.setError("")}>
            <X />
          </IconButton>
        </div>
      )}
      {state.status.errors.length > 0 && !error && (
        <div className="status-error" role="status">
          {state.status.errors.at(-1)?.[1]}
        </div>
      )}
      {dialog === "settings" && (
        <Settings
          ui={ui}
          onClose={() => setDialog("")}
          onAccount={(a) => {
            setEditing(a);
            setDialog("account");
          }}
        />
      )}
      {dialog === "account" && (
        <AccountDialog
          ui={ui}
          account={editing}
          onClose={() => setDialog("")}
        />
      )}
      {dialog === "contacts" && (
        <Contacts
          ui={ui}
          onTimeline={(contact) => showTimeline(contact, "contacts")}
          onClose={() => setDialog("")}
          onWrite={(email) => {
            setDialog("");
            void newCompose({ to: email });
          }}
        />
      )}
      {dialog === "timeline" && timelineContact && (
        <ContactTimeline
          ui={ui}
          contact={timelineContact}
          onClose={() => setDialog(timelineBack)}
          onWrite={(email) => {
            setDialog("");
            void newCompose({ to: email });
          }}
          onOpen={async (message) => {
            const scope = await run("view.scope", {
              account: message.key.account,
              folder: message.key.folder,
            });
            if (scope === undefined)
              throw new Error(t("无法打开邮件所在文件夹"));
            const selected = await run("view.select", {
              key: `${message.key.account}/${message.key.uid}/${message.key.folder}`,
            });
            if (selected === undefined) throw new Error(t("无法打开往来邮件"));
            setDialog("");
            setMode("mail");
            setQuery("");
            setCollapsed((current) =>
              current.filter((id) => id !== message.key.account),
            );
          }}
        />
      )}
      {dialog === "folder" && (
        <FolderDialog ui={ui} onClose={() => setDialog("")} />
      )}
      {junkPanel && <JunkDialog ui={ui} onClose={() => setJunkPanel(false)} />}
      {dialog === "invoice" && (
        <InvoiceDialog ui={ui} onClose={() => setDialog("")} />
      )}
      {compose && !nativeBridge && (
        <ComposeView
          key={compose.id}
          compose={compose}
          ui={ui}
          onClose={() => setComposeID(null)}
        />
      )}
    </main>
  );
}
