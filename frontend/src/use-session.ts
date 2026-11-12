import { setLanguage } from "./i18n";
import { useCallback, useEffect, useRef, useState } from "react";
import { useMessageDeletions } from "./use-message-deletions";
import { execute, snapshot } from "./api";
import type { Params, Snapshot } from "./types";
// FolderDeletion 将确认后的即时反馈保留到该账户后台处理及同步结束。
interface FolderDeletion {
  account: string;
  folder: string;
  submitted: boolean;
  revision?: number;
  errorRevision: number;
}
// useSession 用串行轮询同步多窗口快照，清理时停止所有定时器。
export function useSession() {
  const [state, setState] = useState<Snapshot | null>(null);
  const [error, setError] = useState("");
  const [openingMessage, setOpeningMessage] = useState(false);
  const openingSequence = useRef(0);
  const [folderDeletions, setFolderDeletions] = useState<FolderDeletion[]>([]);
  const [deleteFeedback, setDeleteFeedback] = useState<{
    error?: string;
  } | null>(null);
  const messageDeletes = useMessageDeletions(state);
  const latest = useRef<Snapshot | null>(null);
  const revision = useRef(-1);
  const active = useRef(true);
  const refresh = useCallback(async () => {
    const next = await snapshot();
    setLanguage(next.prefs.language);
    if (active.current && revision.current < next.revision) {
      revision.current = next.revision;
      latest.current = next;
      setState(next);
    }
    return next;
  }, []);
  useEffect(() => {
    active.current = true;
    let timer: ReturnType<typeof setTimeout>;
    let cancelled = false;
    const poll = async () => {
      try {
        await refresh();
      } catch (e) {
        if (active.current) setError(String(e));
      }
      if (!cancelled) timer = setTimeout(poll, 500);
    };
    void poll();
    return () => {
      active.current = false;
      cancelled = true;
      clearTimeout(timer);
    };
  }, [refresh]);
  useEffect(() => {
    if (!state) return;
    const finished = folderDeletions.filter(
      (item) =>
        item.submitted &&
        state.revision >= (item.revision || 0) &&
        !state.status.busyAccounts.includes(item.account),
    );
    if (finished.length === 0) return;
    const lastError = state.status.errors.at(-1);
    const failure =
      lastError &&
      finished.some(
        (item) =>
          state.status.errorRevision > item.errorRevision &&
          (lastError[0] === `folder.mutate:${item.account}` ||
            lastError[0] === item.account),
      )
        ? lastError
        : undefined;
    setFolderDeletions((items) =>
      items.filter((item) => !finished.includes(item)),
    );
    setDeleteFeedback(failure ? { error: failure[1] } : {});
  }, [state, folderDeletions]);
  const run = useCallback(
    async <T = unknown>(
      id: string,
      params: Params = {},
    ): Promise<T | undefined> => {
      setError("");
      const opens =
        (id === "view.select" &&
          params.open !== false &&
          (!Array.isArray(params.keys) || params.keys.length > 0)) ||
        [
          "message.open",
          "view.selectGroup",
          "view.next",
          "view.prev",
          "view.groupNext",
          "view.groupPrev",
        ].includes(id);
      const openingTicket = opens ? ++openingSequence.current : 0;
      let openingStarted = performance.now();
      if (opens) setOpeningMessage(true);
      if (id === "view.scope" || id === "view.search") {
        openingSequence.current++;
        setOpeningMessage(false);
      }
      const requestedKeys =
        id === "message.delete"
          ? Array.isArray(params.keys)
            ? params.keys.filter(
                (key): key is string => typeof key === "string",
              )
            : latest.current?.view.selected || []
          : [];
      const messageDeletion =
        id === "message.delete" ? messageDeletes.begin(requestedKeys) : null;
      if (
        id === "message.delete" &&
        requestedKeys.length > 0 &&
        !messageDeletion
      )
        return undefined;
      const commandParams = messageDeletion
        ? { ...params, keys: messageDeletion.keys }
        : params;
      const deletion =
        id === "folder.deleteMessages" &&
        typeof params.account === "string" &&
        typeof params.folder === "string"
          ? {
              account: params.account,
              folder: params.folder,
              submitted: false,
              errorRevision: latest.current?.status.errorRevision || 0,
            }
          : null;
      if (messageDeletion) setDeleteFeedback(null);
      if (deletion) {
        messageDeletes.clearFeedback();
        setFolderDeletions((items) => [...items, deletion]);
        setDeleteFeedback(null);
      }
      let messageAccepted = false;
      try {
        if (opens && document.visibilityState === "visible") {
          // 先给浏览器绘制 loading 的机会，再执行可能触发大快照渲染的命令。
          await new Promise<void>((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
          );
          if (openingTicket !== openingSequence.current) return undefined;
          openingStarted = performance.now();
        }
        const result = await execute<T>(id, commandParams);
        if (messageDeletion) {
          const task =
            typeof result === "object" &&
            result !== null &&
            "task" in result &&
            typeof result.task === "string"
              ? result.task
              : undefined;
          const acceptedRevision =
            typeof result === "object" &&
            result !== null &&
            "revision" in result &&
            typeof result.revision === "number"
              ? result.revision
              : undefined;
          messageDeletes.submitted(messageDeletion.id, task, acceptedRevision);
          messageAccepted = true;
        }
        await refresh();
        if (deletion)
          setFolderDeletions((items) =>
            items.map((item) =>
              item === deletion
                ? {
                    ...item,
                    submitted: true,
                    revision: latest.current?.revision,
                  }
                : item,
            ),
          );
        return result;
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e);
        if (messageDeletion) {
          // 命令已受理时，快照暂时失败不能误判为删除失败；后续轮询仍会完成核对。
          if (!messageAccepted)
            messageDeletes.rejected(messageDeletion.id, message);
        } else if (!deletion) setError(message);
        if (deletion) {
          setFolderDeletions((items) =>
            items.filter((item) => item !== deletion),
          );
          setDeleteFeedback({ error: message });
        }
        return undefined;
      } finally {
        if (openingTicket && openingTicket === openingSequence.current) {
          // 缓存命令可能在首帧前完成，保留 200ms 可见过渡，避免 loading 从未被绘制。
          const remaining = 200 - (performance.now() - openingStarted);
          if (remaining > 0)
            await new Promise<void>((resolve) =>
              setTimeout(resolve, remaining),
            );
          if (active.current && openingTicket === openingSequence.current)
            setOpeningMessage(false);
        }
      }
    },
    [
      refresh,
      messageDeletes.begin,
      messageDeletes.submitted,
      messageDeletes.rejected,
      messageDeletes.clearFeedback,
    ],
  );
  return {
    state,
    openingMessage,
    error,
    setError,
    run,
    refresh,
    folderDeletions,
    changingFolders: folderDeletions.filter(
      (item) =>
        !item.submitted ||
        !state ||
        state.revision < (item.revision || 0) ||
        state.status.busyTasks.includes(`folder.mutate:${item.account}`),
    ),
    deleteFeedback,
    messageDeletions: messageDeletes.messageDeletions,
    messageDeleteFeedback: messageDeletes.messageDeleteFeedback,
    deletingKeys: messageDeletes.deletingKeys,
    deletingFolders: messageDeletes.deletingFolders,
  };
}
export type SessionUI = ReturnType<typeof useSession>;
