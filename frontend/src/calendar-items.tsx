import { useId, useState, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";
import type { CalendarDay, CalendarEvent } from "./types";

// SOURCE_COLORS 沿用旧版按账户顺序分配的颜色，筛选来源不会改变颜色。
export const SOURCE_COLORS = [
  "#34c759",
  "#f0883e",
  "#3478f6",
  "#af52de",
  "#ff2d55",
  "#30b0c7",
];

// CalendarSource 关联系统日历颜色和来源标识。
export interface CalendarSource {
  id: string;
  title: string;
  color: string;
  source?: string;
}

// calendarColor 只接受日历返回的十六进制颜色，无颜色时使用旧版调色板。
export function calendarColor(source: CalendarSource, index: number) {
  return /^#[0-9a-f]{6}$/i.test(source.color)
    ? source.color
    : SOURCE_COLORS[(index + 2) % SOURCE_COLORS.length];
}

// sourceStyle 将单一来源色传给圆点、竖线及背景，保持明暗主题一致。
export function sourceStyle(color: string): CSSProperties {
  return { "--source-color": color } as CSSProperties;
}

// CalendarItem 将悬停提示挂到文档层，避免被网格滚动区截断；键盘聚焦同样显示。
function CalendarItem({
  className,
  color,
  hint,
  children,
  onClick,
}: {
  className: string;
  color: string;
  hint: string;
  children: ReactNode;
  onClick: () => void;
}) {
  const id = useId();
  const [anchor, setAnchor] = useState<{
    left: number;
    top: number;
    above: boolean;
  } | null>(null);
  const show = (node: HTMLElement) => {
    const r = node.getBoundingClientRect(),
      above = r.bottom > window.innerHeight - 155;
    setAnchor({
      left: Math.max(8, Math.min(r.left, window.innerWidth - 328)),
      top: above ? r.top - 7 : r.bottom + 7,
      above,
    });
  };
  return (
    <>
      <button
        className={className}
        style={sourceStyle(color)}
        aria-describedby={anchor ? id : undefined}
        onMouseEnter={(e) => show(e.currentTarget)}
        onMouseLeave={() => setAnchor(null)}
        onFocus={(e) => show(e.currentTarget)}
        onBlur={() => setAnchor(null)}
        onClick={() => {
          setAnchor(null);
          onClick();
        }}
      >
        {children}
      </button>
      {anchor &&
        createPortal(
          <div
            id={id}
            role="tooltip"
            className="calendar-tooltip"
            style={{
              left: anchor.left,
              top: anchor.top,
              transform: anchor.above ? "translateY(-100%)" : undefined,
            }}
          >
            {hint}
          </div>,
          document.body,
        )}
    </>
  );
}

// CalendarMail 用账户色圆点区分邮件来源，提示完整主题、发件人、时间和邮箱。
export function CalendarMail({
  mail,
  color,
  account,
  en,
  onClick,
}: {
  mail: CalendarDay["messages"][number];
  color: string;
  account: string;
  en: boolean;
  onClick: () => void;
}) {
  const subject = mail.subject.trim() || (en ? "(no subject)" : "（无主题）");
  const sender = mail.from.name
    ? `${mail.from.name} <${mail.from.email}>`
    : mail.from.email;
  const hint = `${subject}\n${sender}\n${new Date(mail.date * 1000).toLocaleString(en ? "en" : "zh-CN")}\n${account}`;
  return (
    <CalendarItem
      className={`calendar-mail ${mail.unread ? "unread" : ""}`}
      color={color}
      hint={hint}
      onClick={onClick}
    >
      <span className="calendar-mail-dot" aria-hidden="true" />
      <span className="calendar-item-title">{subject}</span>
    </CalendarItem>
  );
}

// calendarEventWhen 格式化完整日程时间；全天结束时间采用排他的下一日。
export function calendarEventWhen(event: CalendarEvent, en: boolean) {
  const locale = en ? "en" : "zh-CN";
  const start = new Date(event.start * 1000),
    end = new Date(event.end * 1000);
  if (event.allDay) {
    const last = new Date(Math.max(event.start, event.end - 1) * 1000);
    return `${en ? "All day" : "全天"} · ${start.toLocaleDateString(locale)}${start.toDateString() !== last.toDateString() ? ` – ${last.toLocaleDateString(locale)}` : ""}`;
  }
  return `${start.toLocaleString(locale)} – ${end.toLocaleString(locale)}`;
}

// CalendarSchedule 用日历来源色竖线和背景显示日程，定时事件显示开始时间。
export function CalendarSchedule({
  event,
  source,
  color,
  en,
  onClick,
}: {
  event: CalendarEvent;
  source: string;
  color: string;
  en: boolean;
  onClick: () => void;
}) {
  const locale = en ? "en" : "zh-CN";
  const start = new Date(event.start * 1000);
  const time = start.toLocaleTimeString(locale, {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
  const when = calendarEventWhen(event, en);
  const hint = [event.title, when, source, event.location]
    .filter(Boolean)
    .join("\n");
  return (
    <CalendarItem
      className={`calendar-event ${event.allDay ? "all-day" : "timed"}`}
      color={color}
      hint={hint}
      onClick={onClick}
    >
      <span className="calendar-item-title">
        {!event.allDay && <time>{time} </time>}
        {event.title}
      </span>
    </CalendarItem>
  );
}
