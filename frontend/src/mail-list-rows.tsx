import {
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { Flag, Paperclip } from "lucide-react";
import { t } from "./i18n";
import { dateLabel } from "./components";
import { keyString, type Row } from "./types";
import { rowIdentity } from "./use-deletion-rows";
import { listScrollAnchor, listWindow } from "./mail-list-window";
// RowItem 保留多选、未读、红旗、主题组与附件的可见反馈。
function RowItem({
  row,
  selected,
  english,
  compact,
  onSelect,
  selecting,
  checked,
  disabled,
  onToggle,
  exiting,
  delay,
}: {
  row: Row;
  selected: boolean;
  english: boolean;
  compact: boolean;
  onSelect: (extend: boolean) => void;
  selecting: boolean;
  checked: boolean;
  disabled: boolean;
  onToggle: (checked: boolean) => void;
  exiting: boolean;
  delay: number;
}) {
  const m = row.message;
  const countLabel = english
    ? `${row.unread} unread / ${row.count} total`
    : `${row.unread} 封未读 / 共 ${row.count} 封`;
  return (
    <div
      className={`mail-row-container ${selecting ? "selecting" : ""} ${exiting ? "deleting" : ""}`}
      data-deleting={exiting ? "true" : undefined}
      style={exiting ? { animationDelay: `${delay}ms` } : undefined}
    >
      {selecting && (
        <input
          type="checkbox"
          className="mail-row-check"
          data-testid={`list.check.${keyString(m.key)}`}
          aria-label={`${t("选择邮件")}: ${m.subject || t("（无主题）")}${row.count > 1 ? ` (${row.count})` : ""}`}
          checked={checked}
          ref={(node) => {
            if (node) node.indeterminate = selected && !checked;
          }}
          disabled={disabled}
          onChange={(event) => onToggle(event.target.checked)}
        />
      )}
      <button
        className={`mail-row ${selected ? "selected" : ""} ${row.unread ? "unread" : ""}`}
        aria-pressed={selected}
        disabled={disabled}
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
                  (english
                    ? "Open to download message"
                    : t("打开查看邮件内容"))}
              </span>
              {m.hasAttachments && <Paperclip size={12} />}
            </div>
          )}
        </div>
      </button>
    </div>
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

// ListRow 保留删除退场投影，虚拟窗口不改变邮件选择与后台删除状态。
interface ListRow {
  row: Row;
  exiting: boolean;
  delay: number;
}

// MailListRows 按实际视口高度渲染约两屏邮件；完整元数据仍供搜索、全选和键盘导航使用。
export function MailListRows({
  items,
  selected,
  compact,
  english,
  selecting,
  disabled,
  scope,
  focused,
  group,
  onToggle,
  onSelect,
  children,
}: {
  items: ListRow[];
  selected: Set<string>;
  compact: boolean;
  english: boolean;
  selecting: boolean;
  disabled: boolean;
  scope: string;
  focused: string | null;
  group: string | null;
  onToggle: (row: Row, checked: boolean) => void;
  onSelect: (row: Row, extend: boolean) => void;
  children: ReactNode;
}) {
  const scroll = useRef<HTMLDivElement>(null);
  const [viewport, setViewport] = useState({
    top: 0,
    height: 0,
    row: compact ? 48 : 94,
    date: 22,
  });
  const previous = useRef<{
    scope: string;
    keys: string[];
    offsets: number[];
    top: number;
  } | null>(null);
  const navigation = useRef("");
  const layout = useMemo(() => {
    const offsets = [0];
    const keys: string[] = [];
    let lastDate = "";
    const entries = items.map((item) => {
      const bucket = dateGroup(item.row.message.date);
      const header = bucket !== lastDate ? bucket : "";
      lastDate = bucket;
      const height = viewport.row + (header ? viewport.date : 0);
      offsets.push(offsets[offsets.length - 1] + height);
      keys.push(rowIdentity(item.row));
      return { item, header, height };
    });
    return { entries, offsets, keys };
  }, [items, viewport.row, viewport.date]);
  useLayoutEffect(() => {
    const element = scroll.current;
    if (!element) return;
    // 直接测量剩余高度，窗口缩放及批量工具栏出现时无需假定每屏固定行数。
    const measure = () => {
      const style = getComputedStyle(element);
      const row =
        parseFloat(style.getPropertyValue("--mail-row-height")) ||
        (compact ? 48 : 94);
      const date =
        parseFloat(style.getPropertyValue("--mail-date-height")) || 22;
      setViewport((old) =>
        old.height === element.clientHeight &&
        old.top === element.scrollTop &&
        old.row === row &&
        old.date === date
          ? old
          : { top: element.scrollTop, height: element.clientHeight, row, date },
      );
    };
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    measure();
    return () => observer.disconnect();
  }, [compact]);
  useLayoutEffect(() => {
    const element = scroll.current;
    if (!element) return;
    const old = previous.current;
    let top = element.scrollTop;
    const destination = `${group || ""}\0${focused || ""}`;
    if (old?.scope !== scope) {
      top = 0;
      // 搜索可能保留正在阅读的邮件，不能把新列表又拉回旧阅读位置。
      navigation.current = old ? destination : "";
    } else if (old.keys !== layout.keys) {
      top = listScrollAnchor(
        old.keys,
        old.offsets,
        layout.keys,
        layout.offsets,
        old.top,
      );
    }
    if (destination !== navigation.current) {
      const index = group
        ? layout.keys.indexOf(`group:${group}`)
        : focused
          ? items.findIndex(({ row }) => row.keys.includes(focused))
          : -1;
      if (index >= 0) {
        const start = layout.offsets[index],
          end = layout.offsets[index + 1];
        if (start < top) top = start;
        else if (end > top + element.clientHeight)
          top = end - element.clientHeight;
        navigation.current = destination;
      } else if (!focused && !group) navigation.current = destination;
    }
    element.scrollTop = Math.max(
      0,
      Math.min(
        top,
        Math.max(0, (layout.offsets.at(-1) || 0) - element.clientHeight),
      ),
    );
    previous.current = {
      scope,
      keys: layout.keys,
      offsets: layout.offsets,
      top: element.scrollTop,
    };
    setViewport((old) =>
      old.top === element.scrollTop && old.height === element.clientHeight
        ? old
        : { ...old, top: element.scrollTop, height: element.clientHeight },
    );
  }, [layout, scope, focused, group, items]);
  const range = listWindow(layout.offsets, viewport.top, viewport.height);
  return (
    <div
      className="list-scroll"
      ref={scroll}
      tabIndex={0}
      role="list"
      aria-label={english ? "Message rows" : t("邮件列表")}
      data-testid="list.scroll"
      onScroll={(event) => {
        const element = event.currentTarget;
        if (previous.current) previous.current.top = element.scrollTop;
        // 聚焦行移出虚拟窗口前把焦点留在列表，保持多选快捷键可用。
        if (
          element.contains(document.activeElement) &&
          document.activeElement !== element
        )
          element.focus({ preventScroll: true });
        setViewport((old) =>
          old.top === element.scrollTop
            ? old
            : { ...old, top: element.scrollTop },
        );
      }}
    >
      <div
        aria-hidden="true"
        className="list-spacer"
        style={{ height: range.before }}
      />
      {layout.entries
        .slice(range.start, range.end)
        .map(({ item, header, height }, offset) => {
          const { row, exiting, delay } = item;
          const index = range.start + offset;
          return (
            <div
              key={layout.keys[index]}
              role="listitem"
              aria-posinset={index + 1}
              aria-setsize={items.length}
              className="mail-list-slot"
              style={{ height }}
            >
              {header && <div className="date-group">{t(header)}</div>}
              <RowItem
                row={row}
                compact={compact}
                english={english}
                selected={exiting || row.keys.some((key) => selected.has(key))}
                checked={exiting || row.keys.every((key) => selected.has(key))}
                selecting={selecting}
                disabled={exiting || disabled}
                exiting={exiting}
                delay={delay}
                onToggle={(checked) => onToggle(row, checked)}
                onSelect={(extend) => onSelect(row, extend)}
              />
            </div>
          );
        })}
      <div
        aria-hidden="true"
        className="list-spacer"
        style={{ height: range.after }}
      />
      {children}
    </div>
  );
}
