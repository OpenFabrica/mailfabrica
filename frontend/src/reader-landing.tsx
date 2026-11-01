import { t } from "./i18n";
import { Calendar, Flag, Mail, Search, SquarePen, Users } from "lucide-react";
import type { SessionUI } from "./use-session";

// 欢迎页上可跳转的入口，由 App 提供实际的导航和写信逻辑。
export interface LandingActions {
  onCompose: () => void;
  onScope: (kind: string) => void;
  onCalendar: () => void;
  onContacts: () => void;
  onSearch: () => void;
}

const mac = /Mac/.test(navigator.userAgent);
const mod = mac ? "⌘" : "Ctrl+";

// greeting 按本地时间返回问候语。
function greeting(hour: number) {
  if (hour < 6) return t("夜深了");
  if (hour < 11) return t("早上好");
  if (hour < 13) return t("中午好");
  if (hour < 18) return t("下午好");
  return t("晚上好");
}

// todayEvents 统计已加载日历中与今天重叠的可见日程；未加载或不含今天时返回 null。
function todayEvents(ui: SessionUI) {
  const calendar = ui.state?.calendar;
  if (!calendar?.supported || !calendar.events) return null;
  const now = new Date();
  const day = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
  // 只用已经显式加载过的数据，不在这里触发系统日历权限请求
  if (!calendar.since || calendar.since > day || calendar.until < day)
    return null;
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const from = start.getTime() / 1000;
  const to = from + 86400;
  const hidden = (ui.state?.prefs.calendarHiddenCalendars as string[]) || [];
  return calendar.events.filter(
    (e) => !hidden.includes(e.calendar) && e.start < to && e.end > from,
  ).length;
}

// ReaderLanding 是未选择邮件时阅读区的欢迎页：问候、今日概览、快捷入口和快捷键提示。
export function ReaderLanding({
  ui,
  actions,
}: {
  ui: SessionUI;
  actions: LandingActions;
}) {
  const state = ui.state;
  if (!state) return null;
  const en = state.prefs.language === "en";
  const now = new Date();
  const date = now.toLocaleDateString(en ? "en-US" : "zh-CN", {
    month: "long",
    day: "numeric",
    weekday: "long",
  });
  const { unread, flagged } = state.counts;
  const events = todayEvents(ui);
  const tiles = [
    ["compose", SquarePen, t("写邮件"), `${mod}N`, actions.onCompose],
    ["unread", Mail, t("未读邮件"), "", () => actions.onScope("unread")],
    ["flagged", Flag, t("红旗邮件"), "", () => actions.onScope("flagged")],
    ["calendar", Calendar, t("日历"), "", actions.onCalendar],
    ["contacts", Users, t("联系人"), "", actions.onContacts],
    ["search", Search, t("搜索"), `${mod}K`, actions.onSearch],
  ] as const;
  // 进场动画按 --d 错峰，序号从 0 开始依次递增
  let d = 0;
  const step = () => ({ "--d": d++ }) as React.CSSProperties;
  return (
    <section className="reader reader-landing" data-testid="reader.landing">
      <div className="landing-mark" style={step()}>
        <Mail size={30} strokeWidth={1.6} />
      </div>
      <div className="landing-hello" style={step()}>
        <h2>{greeting(now.getHours())}</h2>
        <p>{date}</p>
      </div>
      <div className="landing-pills" style={step()}>
        <button
          data-testid="landing.unread"
          onClick={() => actions.onScope("unread")}
        >
          <Mail size={13} />
          {en ? (
            <>
              <b>{unread}</b> unread
            </>
          ) : (
            <>
              <b>{unread}</b>封未读
            </>
          )}
        </button>
        <button
          data-testid="landing.flagged"
          onClick={() => actions.onScope("flagged")}
        >
          <Flag size={13} />
          {en ? (
            <>
              <b>{flagged}</b> flagged
            </>
          ) : (
            <>
              <b>{flagged}</b>封红旗
            </>
          )}
        </button>
        {events !== null && (
          <button data-testid="landing.events" onClick={actions.onCalendar}>
            <Calendar size={13} />
            {en ? (
              <>
                <b>{events}</b> {events === 1 ? "event" : "events"} today
              </>
            ) : (
              <>
                今天<b>{events}</b>个日程
              </>
            )}
          </button>
        )}
      </div>
      <div className="landing-tiles">
        {tiles.map(([id, Icon, label, key, onClick]) => (
          <button
            key={id}
            data-testid={`landing.${id}`}
            style={step()}
            onClick={onClick}
          >
            <div>
              <span>
                <Icon size={16} />
              </span>
              {key && <kbd>{key}</kbd>}
            </div>
            <strong>{label}</strong>
          </button>
        ))}
      </div>
      <div className="landing-hints" style={step()}>
        <span>
          <kbd>↑</kbd>
          <kbd>↓</kbd>
          {t("切换邮件")}
        </span>
        <span>
          <kbd>{mod}R</kbd>
          {t("回复")}
        </span>
        <span>
          <kbd>{mac ? "⌫" : "Del"}</kbd>
          {t("删除")}
        </span>
        <span>
          <kbd>F5</kbd>
          {t("同步")}
        </span>
      </div>
    </section>
  );
}
