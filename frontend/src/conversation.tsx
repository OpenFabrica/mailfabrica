import {
  ChevronUp,
  ChevronDown,
  Layers,
  Sparkles,
  LoaderCircle,
} from "lucide-react";
import type { Row, Snapshot } from "./types";
import type { SessionUI } from "./use-session";

// ConversationSummary 用快照展示完整组汇总；用户点击才调用已有 AI 配置。
export function ConversationSummary({
  row,
  ui,
  toolbar = false,
}: {
  row: Row;
  ui: SessionUI;
  toolbar?: boolean;
}) {
  const en = ui.state?.prefs.language === "en";
  const summary = ui.state?.aiGroupSummary;
  const current = summary?.group === row.group ? summary : null;
  const changed =
    current &&
    (current.keys.length !== row.keys.length ||
      current.keys.some((k) => !row.keys.includes(k)));
  return (
    <>
      <button
        className={`conversation-ai ${toolbar ? "" : "text-button"}`}
        disabled={!!summary?.busy}
        onClick={() => void ui.run("ai.summarizeGroup", { group: row.group })}
      >
        {current?.busy ? (
          <LoaderCircle size={14} className="spin" />
        ) : (
          <Sparkles size={14} />
        )}
        {current?.busy
          ? en
            ? "Summarizing…"
            : "正在汇总…"
          : en
            ? "Summarize conversation"
            : "一键 AI 汇总"}
      </button>
      {!toolbar && current && (
        <SummaryResult summary={current} changed={!!changed} en={!!en} />
      )}
    </>
  );
}

// SummaryResult 将 AI 生成内容和来源封数标清，与邮件原文分开显示。
export function SummaryResult({
  summary,
  changed,
  en,
}: {
  summary: NonNullable<Snapshot["aiGroupSummary"]>;
  changed: boolean;
  en: boolean;
}) {
  if (!summary.text && !summary.error && !summary.busy) return null;
  return (
    <aside
      className="conversation-summary"
      aria-label={en ? "Conversation AI summary" : "聚合邮件 AI 汇总"}
    >
      <strong>
        {en ? "AI summary" : "AI 汇总"} · {summary.keys.length}{" "}
        {en ? "messages" : "封邮件"}
      </strong>
      {changed && (
        <p className="muted">
          {en
            ? "Conversation changed. Generate a new summary."
            : "聚合邮件已变化，请重新汇总。"}
        </p>
      )}
      {summary.busy && (
        <p role="status">
          {en
            ? "Reading messages and generating summary…"
            : "正在读取组内邮件并生成汇总…"}
        </p>
      )}
      {summary.error && (
        <p role="alert" className="inline-error">
          {summary.error}
        </p>
      )}
      {summary.text && <div className="summary-text">{summary.text}</div>}
    </aside>
  );
}

// ConversationToolbar 仅对聚合邮件显示专用上下导航，普通列表箭头仍单独保留。
export function ConversationToolbar({ row, ui }: { row: Row; ui: SessionUI }) {
  const en = ui.state?.prefs.language === "en";
  const focused = ui.state?.view.focused;
  const index = focused ? row.keys.indexOf(focused) : -1;
  return (
    <div
      className="conversation-toolbar"
      aria-label={en ? "Conversation navigation" : "聚合邮件导航"}
    >
      <button
        className="text-button"
        onClick={() => void ui.run("view.selectGroup", { group: row.group })}
      >
        <Layers size={14} />
        {en ? "Conversation" : "聚合邮件"}
      </button>
      <span className="conversation-position" aria-live="polite">
        {index + 1} / {row.keys.length}
      </span>
      <button
        disabled={index <= 0}
        aria-label={en ? "Previous in conversation" : "组内上一封"}
        onClick={() => void ui.run("view.groupPrev")}
      >
        <ChevronUp size={14} />
        {en ? "Previous in group" : "组内上一封"}
      </button>
      <button
        disabled={index < 0 || index >= row.keys.length - 1}
        aria-label={en ? "Next in conversation" : "组内下一封"}
        onClick={() => void ui.run("view.groupNext")}
      >
        <ChevronDown size={14} />
        {en ? "Next in group" : "组内下一封"}
      </button>
      <span className="grow" />
      <ConversationSummary row={row} ui={ui} toolbar />
    </div>
  );
}
