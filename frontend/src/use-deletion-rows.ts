import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { keyString, type Row, type Snapshot } from "./types";
import type { MessageDeletion } from "./use-message-deletions";

// rowIdentity 保持主题组身份稳定，删除旧成员不会把后来到达的新邮件当作已删除成员。
export const rowIdentity = (row: Row) =>
  row.group ? `group:${row.group}` : `message:${row.keys[0]}`;

// ExitRow 在服务器快速返回时保留短暂的退场画面，最长 1.2 秒后释放。
interface ExitRow {
  row: Row;
  scope: string;
  job: number;
  until: number;
  delay: number;
}

// useDeletionRows 投影待删除邮件；整行交错退场，部分组仅扣除选中成员，失败按服务器结果恢复。
export function useDeletionRows(
  state: Snapshot | null,
  jobs: MessageDeletion[],
  feedback: { id: number; error?: string } | null,
) {
  const scope = JSON.stringify([state?.view.scope, state?.view.query]);
  const seen = useRef(new Set<number>());
  const [exits, setExits] = useState<ExitRow[]>([]);
  // 同一帧先投影退场占位，避免子列表在父布局效果保存缓存前误判行已删除而移动滚动锚点。
  const additions: ExitRow[] = [];
  for (const job of jobs) {
    if (seen.current.has(job.id)) continue;
    if (Date.now() >= job.startedAt + 1200) continue;
    const targets = new Set(job.keys);
    const rows = (state?.view.rows || []).filter((row) =>
      row.keys.every((key) => targets.has(key)),
    );
    rows.forEach((row, index) =>
      additions.push({
        row,
        scope,
        job: job.id,
        until: job.startedAt + 1200,
        delay: (900 * index) / Math.max(rows.length - 1, 1),
      }),
    );
  }
  useLayoutEffect(() => {
    if (!state) return;
    const activeIds = new Set(jobs.map((job) => job.id));
    seen.current = activeIds;
    setExits((old) => {
      const kept = old.filter(
        (exit) =>
          exit.scope === scope &&
          !(feedback?.error && feedback.id === exit.job),
      );
      return additions.length || kept.length !== old.length
        ? [...kept, ...additions]
        : old;
    });
  }, [state, jobs, scope, feedback]);
  useEffect(() => {
    if (!exits.length) return;
    const timer = setTimeout(
      () =>
        setExits((items) => items.filter((item) => item.until > Date.now())),
      Math.max(0, Math.min(...exits.map((item) => item.until)) - Date.now()) +
        1,
    );
    return () => clearTimeout(timer);
  }, [exits]);
  const activeExits = [...exits, ...additions].filter(
    (exit) =>
      exit.scope === scope && !(feedback?.error && feedback.id === exit.job),
  );
  const exiting = new Map(
    activeExits.map((exit) => [rowIdentity(exit.row), exit]),
  );
  const deletingKeys = new Set([
    ...jobs.flatMap((job) => job.keys),
    ...activeExits.flatMap((exit) => exit.row.keys),
  ]);
  const messages = new Map(
    state?.view.messages.map((message) => [keyString(message.key), message]) ||
      [],
  );
  const source = state?.view.rows || [];
  const sourceIds = new Set(source.map(rowIdentity));
  const items: { row: Row; exiting: boolean; delay: number }[] = [];
  for (const row of source) {
    const keys = row.keys.filter((key) => !deletingKeys.has(key));
    if (!keys.length) {
      const exit = exiting.get(rowIdentity(row));
      if (exit) items.push({ row: exit.row, exiting: true, delay: exit.delay });
    } else if (keys.length === row.keys.length)
      items.push({ row, exiting: false, delay: 0 });
    else {
      const remaining = keys.flatMap((key) => messages.get(key) || []);
      items.push({
        row: {
          ...row,
          keys,
          count: keys.length,
          unread: remaining.filter((m) => !m.flags.seen).length,
          flagged: remaining.some((m) => m.flags.flagged),
          message: remaining[0] || row.message,
          senders: [
            ...new Set(remaining.map((m) => m.from.name || m.from.email)),
          ],
        },
        exiting: false,
        delay: 0,
      });
    }
  }
  for (const exit of activeExits)
    if (!sourceIds.has(rowIdentity(exit.row)))
      items.push({ row: exit.row, exiting: true, delay: exit.delay });
  items.sort(
    (a, b) =>
      b.row.message.date - a.row.message.date ||
      b.row.message.key.uid - a.row.message.key.uid,
  );
  return {
    items,
    deletingKeys,
    active: jobs.length > 0 || activeExits.length > 0,
    visibleCount: [...messages.keys()].filter((key) => !deletingKeys.has(key))
      .length,
  };
}
