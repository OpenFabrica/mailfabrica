import { t } from "./i18n";
import { useEffect, useRef, useState } from "react";
import { ChevronLeft, ChevronRight, RefreshCw } from "lucide-react";
import { execute } from "./api";
import type { CalendarDay } from "./types";
import type { SessionUI } from "./use-session";
import { IconButton, Dialog } from "./components";
import {
  CalendarMail,
  CalendarSchedule,
  SOURCE_COLORS,
  calendarColor,
  calendarEventWhen,
  sourceStyle,
  type CalendarSource,
} from "./calendar-items";
// dateString 以本地日期生成不受 UTC 跨日影响的日期键。
const dateString = (date: Date) =>
  `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
// CalendarView 将系统事件、农历和真实邮件放在同一月视图。
export function CalendarView({
  ui,
  onMail,
}: {
  ui: SessionUI;
  onMail: (key: string) => void;
}) {
  const [month, setMonth] = useState(
    () => new Date(new Date().getFullYear(), new Date().getMonth(), 1),
  );
  const [days, setDays] = useState<CalendarDay[]>([]);
  const [error, setError] = useState("");
  const [hiddenAccounts, setHiddenAccounts] = useState<string[]>(
    (ui.state?.prefs.calendarHiddenAccounts as string[]) || [],
  );
  const [hiddenCalendars, setHiddenCalendars] = useState<string[]>(
    (ui.state?.prefs.calendarHiddenCalendars as string[]) || [],
  );
  const [calendars, setCalendars] = useState<CalendarSource[]>([]);
  const [selectedDate, setSelectedDate] = useState("");
  const [selected, setSelected] = useState<CalendarDay | null>(null);
  const [refreshToken, setRefreshToken] = useState(0);
  const grid = useRef<HTMLDivElement>(null);
  const [rowCapacity, setRowCapacity] = useState(4);
  useEffect(() => {
    const node = grid.current;
    if (!node) return;
    // 按格子实际高度留出日期和“还有 N 项”的空间，避免内容盖住下一周。
    const measure = () => {
      const height =
        node.querySelector(".calendar-day")?.getBoundingClientRect().height ||
        0;
      if (height) setRowCapacity(Math.max(1, Math.floor((height - 38) / 26)));
    };
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    measure();
    return () => observer.disconnect();
  }, [days.length]);
  useEffect(() => {
    void ui.run("app.prefs", {
      calendarHiddenAccounts: hiddenAccounts,
      calendarHiddenCalendars: hiddenCalendars,
    });
  }, [hiddenAccounts, hiddenCalendars]);
  useEffect(() => {
    const change = (event: Event) => {
      const p = (event as CustomEvent).detail;
      if (typeof p.month === "string" && /^\d{4}-\d{2}$/.test(p.month)) {
        const [y, m] = p.month.split("-").map(Number);
        setMonth(new Date(y, m - 1, 1));
      }
      if (typeof p.day === "string" && /^\d{4}-\d{2}-\d{2}$/.test(p.day)) {
        const [y, m] = p.day.split("-").map(Number);
        setMonth(new Date(y, m - 1, 1));
      }
    };
    document.addEventListener("mailfabrica:calendar", change);
    return () => document.removeEventListener("mailfabrica:calendar", change);
  }, []);
  const en = ui.state?.prefs.language === "en";
  const accounts = ui.state?.accounts || [];
  const accountColors = new Map(
    accounts.map((a, i) => [a.id, SOURCE_COLORS[i % SOURCE_COLORS.length]]),
  );
  const calendarColors = new Map(
    calendars.map((c, i) => [c.id, calendarColor(c, i)]),
  );
  const accountEmails = new Map(accounts.map((a) => [a.id, a.email]));
  const calendarTitles = new Map(calendars.map((c) => [c.id, c.title]));
  const today = dateString(new Date());
  const openMail = (key: string) => {
    setSelected(null);
    onMail(key);
  };
  const mailItem = (m: CalendarDay["messages"][number]) => (
    <CalendarMail
      key={m.key}
      mail={m}
      en={en}
      account={accountEmails.get(m.account) || m.account}
      color={accountColors.get(m.account) || "#92929e"}
      onClick={() => openMail(m.key)}
    />
  );
  const eventItem = (e: CalendarDay["events"][number], day: CalendarDay) => (
    <CalendarSchedule
      key={e.id}
      event={e}
      en={en}
      source={calendarTitles.get(e.calendar) || ""}
      color={calendarColors.get(e.calendar) || "#3478f6"}
      onClick={() => {
        setSelectedDate(day.date);
        setSelected(day);
      }}
    />
  );
  const first = new Date(month);
  first.setDate(1 - first.getDay());
  const last = new Date(first);
  last.setDate(first.getDate() + 41);
  const since = dateString(first),
    until = dateString(last);
  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    // 每轮都从引擎取得当前来源，避免系统日历首次加载后过滤仍使用空列表。
    const poll = async (force = false) => {
      try {
        const state = await execute<{ calendars: CalendarSource[] }>(
          "calendar.load",
          { since, until, force },
        );
        const sources = state.calendars || [];
        const result = await execute<{
          days: CalendarDay[];
          calendars: CalendarSource[];
          error: string | null;
        }>("calendar.days", {
          since,
          until,
          accounts: accounts
            .filter((a) => !hiddenAccounts.includes(a.id))
            .map((a) => a.id),
          calendars: sources.length
            ? sources
                .filter((c) => !hiddenCalendars.includes(c.id))
                .map((c) => c.id)
            : undefined,
        });
        if (!disposed) {
          setDays(result.days);
          setCalendars(result.calendars || []);
          setError(result.error || "");
        }
      } catch (e) {
        if (!disposed) setError(String(e));
      }
      if (!disposed) timer = setTimeout(() => void poll(), 2000);
    };
    void poll(refreshToken > 0);
    return () => {
      disposed = true;
      clearTimeout(timer);
    };
  }, [
    since,
    until,
    accounts.map((a) => a.id).join(","),
    hiddenAccounts.join(","),
    hiddenCalendars.join(","),
    refreshToken,
  ]);
  return (
    <div className="calendar-layout">
      <aside className="calendar-sidebar">
        <h2>
          {month.getFullYear()}
          {t("年")}
          {month.getMonth() + 1}
          {t("月")}
        </h2>
        <div className="mini-calendar">
          {(en
            ? ["S", "M", "T", "W", "T", "F", "S"]
            : [t("日"), t("一"), t("二"), t("三"), t("四"), t("五"), t("六")]
          ).map((s, i) => (
            <span key={i}>{s}</span>
          ))}
          {days.map((d) => (
            <button
              className={`${d.date === today ? "today" : ""} ${d.date === selectedDate ? "selected" : ""} ${Number(d.date.slice(5, 7)) !== month.getMonth() + 1 ? "outside" : ""}`}
              key={d.date}
              onClick={() => {
                setSelectedDate(d.date);
                const date = new Date(`${d.date}T12:00:00`);
                setMonth(new Date(date.getFullYear(), date.getMonth(), 1));
              }}
            >
              {Number(d.date.slice(-2))}
            </button>
          ))}
        </div>
        <h3>{en ? "Mail accounts" : t("邮箱")}</h3>
        {ui.state?.accounts.map((a) => (
          <label
            className="calendar-source"
            key={a.id}
            title={a.email}
            style={sourceStyle(accountColors.get(a.id) || "#92929e")}
          >
            <input
              type="checkbox"
              checked={!hiddenAccounts.includes(a.id)}
              onChange={(e) =>
                setHiddenAccounts((v) =>
                  e.target.checked
                    ? v.filter((id) => id !== a.id)
                    : [...v, a.id],
                )
              }
            />
            <span className="calendar-source-name">{a.email}</span>
          </label>
        ))}
        {calendars.length > 0 && (
          <h3>{en ? "System calendars" : t("系统日历")}</h3>
        )}
        {calendars.map((c) => (
          <label
            className="calendar-source"
            key={c.id}
            style={sourceStyle(calendarColors.get(c.id) || "#3478f6")}
          >
            <input
              type="checkbox"
              checked={!hiddenCalendars.includes(c.id)}
              onChange={(e) =>
                setHiddenCalendars((v) =>
                  e.target.checked
                    ? v.filter((id) => id !== c.id)
                    : [...v, c.id],
                )
              }
            />
            <span className="calendar-source-name">{c.title}</span>
          </label>
        ))}
      </aside>
      <section className="calendar-main">
        <header className="pane-heading">
          <h1>
            {month.toLocaleDateString(en ? "en" : "zh-CN", {
              year: "numeric",
              month: "long",
            })}
          </h1>
          <div>
            <IconButton
              label={t("刷新日历")}
              onClick={() => {
                setRefreshToken((v) => v + 1);
              }}
            >
              <RefreshCw />
            </IconButton>
            <IconButton
              label={t("上个月")}
              onClick={() =>
                setMonth((d) => new Date(d.getFullYear(), d.getMonth() - 1, 1))
              }
            >
              <ChevronLeft />
            </IconButton>
            <button
              onClick={() =>
                setMonth(
                  new Date(new Date().getFullYear(), new Date().getMonth(), 1),
                )
              }
            >
              {en ? "Today" : t("今天")}
            </button>
            <IconButton
              label={t("下个月")}
              onClick={() =>
                setMonth((d) => new Date(d.getFullYear(), d.getMonth() + 1, 1))
              }
            >
              <ChevronRight />
            </IconButton>
          </div>
        </header>
        {error && <div className="inline-error">{error}</div>}
        <div className="weekdays">
          {(en
            ? ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
            : [
                t("星期日"),
                t("星期一"),
                t("星期二"),
                t("星期三"),
                t("星期四"),
                t("星期五"),
                t("星期六"),
              ]
          ).map((d) => (
            <span key={d}>{d}</span>
          ))}
        </div>
        <div className="month-grid" ref={grid}>
          {days.map((d) => (
            <div
              key={d.date}
              className={`calendar-day ${Number(d.date.slice(5, 7)) !== month.getMonth() + 1 ? "outside" : ""} ${d.date === today ? "is-today" : ""} ${d.date === selectedDate ? "selected" : ""}`}
              onClick={() => setSelectedDate(d.date)}
              onDoubleClick={() => setSelected(d)}
            >
              <header>
                <span
                  className={d.date === dateString(new Date()) ? "today" : ""}
                >
                  {Number(d.date.slice(-2))}
                </span>
                <small>{en ? "" : d.lunar}</small>
              </header>
              {(() => {
                const total = d.messageCount + d.events.length;
                const visible =
                  total > rowCapacity ? rowCapacity - 1 : rowCapacity;
                const events = d.events.slice(0, visible);
                const mails = d.messages.slice(0, visible - events.length);
                const shown = events.length + mails.length;
                return (
                  <>
                    {events.map((e) => eventItem(e, d))}
                    {mails.map(mailItem)}
                    {total > shown && (
                      <button
                        className="calendar-more text-button muted"
                        onClick={() => setSelected(d)}
                      >
                        {en
                          ? `${total - shown} more`
                          : `还有${total - shown}项`}
                      </button>
                    )}
                  </>
                );
              })()}
            </div>
          ))}
        </div>
      </section>
      {selected && (
        <Dialog title={selected.date} onClose={() => setSelected(null)}>
          <div className="day-detail-items">
            {selected.events.map((e) => (
              <div className="day-event" key={e.id}>
                {eventItem(e, selected)}
                <p>
                  {calendarEventWhen(e, en)}
                  {e.location ? ` · ${e.location}` : ""}
                </p>
              </div>
            ))}
            {selected.messages.map(mailItem)}
          </div>
        </Dialog>
      )}
    </div>
  );
}
