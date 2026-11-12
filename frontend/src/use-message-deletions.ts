import { useCallback, useEffect, useRef, useState } from "react";
import type { Snapshot } from "./types";

// MessageDeletion 记录已确认的邮件键，导航及新邮件到达不会扩大删除范围。
export interface MessageDeletion {
  id: number;
  keys: string[];
  accounts: string[];
  startedAt: number;
  submitted: boolean;
  task?: string;
  revision?: number;
}

// useMessageDeletions 在请求发出前隐藏选中邮件，后台核对完成后释放隐藏并报告结果。
export function useMessageDeletions(state: Snapshot | null) {
  const [jobs, setJobs] = useState<MessageDeletion[]>([]);
  const current = useRef<MessageDeletion[]>([]);
  const sequence = useRef(0);
  const [feedback, setFeedback] = useState<{
    id: number;
    keys: string[];
    error?: string;
  } | null>(null);
  const begin = useCallback((keys: string[]) => {
    const pending = new Set(current.current.flatMap((job) => job.keys));
    const unique = [...new Set(keys)].filter((key) => !pending.has(key));
    if (!unique.length) return null;
    const job: MessageDeletion = {
      id: ++sequence.current,
      keys: unique,
      accounts: [...new Set(unique.map((key) => key.split("/")[0]))],
      startedAt: Date.now(),
      submitted: false,
    };
    current.current = [...current.current, job];
    setJobs(current.current);
    setFeedback(null);
    return job;
  }, []);
  const submitted = useCallback(
    (id: number, task?: string, revision?: number) => {
      current.current = current.current.map((job) =>
        job.id === id ? { ...job, submitted: true, task, revision } : job,
      );
      setJobs(current.current);
    },
    [],
  );
  const rejected = useCallback((id: number, error: string) => {
    const job = current.current.find((item) => item.id === id);
    current.current = current.current.filter((item) => item.id !== id);
    setJobs(current.current);
    if (job) setFeedback({ id, keys: job.keys, error });
  }, []);
  useEffect(() => {
    if (!state) return;
    const finished = current.current.filter(
      (job) =>
        job.submitted &&
        state.revision >= (job.revision || 0) &&
        !job.accounts.some((account) =>
          state.status.busyAccounts.includes(account),
        ),
    );
    if (!finished.length) return;
    const failure = [...state.status.errors]
      .reverse()
      .find(([id]) =>
        finished.some(
          (job) =>
            job.task === id ||
            (job.accounts.includes(id) &&
              state.status.errors.at(-1)?.[0] === id &&
              state.status.errorRevision > (job.revision || 0)),
        ),
      );
    current.current = current.current.filter((job) => !finished.includes(job));
    setJobs(current.current);
    setFeedback({
      id: finished[finished.length - 1].id,
      keys: finished.flatMap((job) => job.keys),
      error: failure?.[1],
    });
  }, [state, jobs]);
  const clearFeedback = useCallback(() => setFeedback(null), []);
  const deletingKeys = new Set(jobs.flatMap((job) => job.keys));
  // 仅服务器变更尚未结束时隐藏角标；目录核对仍忙时也显示已更新的缓存计数。
  const changing = jobs.filter(
    (job) =>
      !job.submitted ||
      !job.task ||
      !state ||
      state.revision < (job.revision || 0) ||
      state.status.busyTasks.includes(job.task),
  );
  const deletingFolders = new Set(
    changing
      .flatMap((job) => job.keys)
      .map((key) => {
        const parts = key.split("/");
        return `${parts[0]}\0${parts.slice(2).join("/")}`;
      }),
  );
  return {
    messageDeletions: jobs,
    messageDeleteFeedback: feedback,
    deletingKeys,
    deletingFolders,
    begin,
    submitted,
    rejected,
    clearFeedback,
  };
}
