import { MailListRows } from "./mail-list-rows";
import { useDeletionRows } from "./use-deletion-rows";
import { t } from "./i18n";
import { createPortal } from "react-dom";
import { useEffect, useState } from "react";
import {
  CheckCheck,
  ChevronDown,
  Inbox,
  ListChecks,
  LoaderCircle,
  Trash2,
} from "lucide-react";
import type { SessionUI } from "./use-session";
import { Dialog } from "./components";
import { useDismiss } from "./use-dismiss";
import { keyString } from "./types";
// MailList 提供分组列表和同一后端搜索范围。
export function MailList({
  ui,
  onCompose,
  onJunk,
}: {
  ui: SessionUI;
  onCompose: () => void;
  onJunk: () => void;
}) {
  const { state, run } = ui;
  const [menu, setMenu] = useState(false);
  const menuRoot = useDismiss(menu, () => setMenu(false));
  const [selecting, setSelecting] = useState(false);
  const [deleteKeys, setDeleteKeys] = useState<string[] | null>(null);
  const [bulkTask, setBulkTask] = useState<{
    accounts: string[];
    awaiting: boolean;
    action: string;
  } | null>(null);
  useEffect(() => {
    setSelecting(false);
    setDeleteKeys(null);
  }, [
    state?.view.scope.kind,
    state?.view.scope.account,
    state?.view.scope.folder,
    state?.view.query,
  ]);
  useEffect(() => {
    if (
      bulkTask &&
      !bulkTask.awaiting &&
      state &&
      !bulkTask.accounts.some((a) => state.status.busyAccounts.includes(a))
    )
      setBulkTask(null);
  }, [bulkTask, state]);
  const deletionRows = useDeletionRows(
    state,
    ui.messageDeletions,
    ui.messageDeleteFeedback,
  );
  if (!state) return null;
  const selected = new Set(
    state.view.selected.filter((key) => !deletionRows.deletingKeys.has(key)),
  );
  const deleteTargets = new Set(deleteKeys || []);
  const visibleKeys = state.view.messages
    .map((m) => keyString(m.key))
    .filter((key) => !deletionRows.deletingKeys.has(key));
  const allSelected =
    visibleKeys.length > 0 && visibleKeys.every((key) => selected.has(key));
  const selectAll = (checked: boolean) => {
    void run("view.select", { keys: checked ? visibleKeys : [], open: false });
  };
  const stopSelecting = () => {
    setSelecting(false);
    void run("view.select", { keys: [], open: false });
  };
  const bulkAction = async (command: string, keys: string[]) => {
    if (command === "message.delete") {
      setDeleteKeys(null);
      void run(command, { keys });
      return;
    }
    const targets = new Set(keys);
    const accounts = [
      ...new Set(
        state.view.messages
          .filter((m) => targets.has(keyString(m.key)))
          .map((m) => m.key.account),
      ),
    ];
    setBulkTask({ accounts, awaiting: true, action: command });
    setDeleteKeys(null);
    const result = await run(command, { keys, read: true });
    if (result === undefined) setBulkTask(null);
    else setBulkTask({ accounts, awaiting: false, action: command });
  };
  const en = state.prefs.language === "en";
  const titles: Record<string, string> = en
    ? {
        allInboxes: "All inboxes",
        unread: "Unread",
        flagged: "Flagged",
        invoices: "Invoices",
      }
    : {
        allInboxes: t("全部收件箱"),
        unread: t("未读邮件"),
        flagged: t("红旗邮件"),
        invoices: t("发票"),
      };
  const compact = state.prefs.mailListDensity !== "comfortable";
  const scope = state.view.scope;
  const title = titles[scope.kind] || scope.folder || "";
  const deleting = ui.folderDeletions.some(
    (item) =>
      scope.kind !== "folder" ||
      (item.account === scope.account && item.folder === scope.folder),
  );
  return (
    <section
      className={`mail-list ${compact ? "compact" : "comfortable"}`}
      aria-label={en ? "Message list" : t("邮件列表")}
      onKeyDown={(event) => {
        if ((event.target as HTMLElement).closest("[role=dialog]")) return;
        if (
          (event.metaKey || event.ctrlKey) &&
          event.key.toLowerCase() === "a"
        ) {
          event.preventDefault();
          event.stopPropagation();
          if (!bulkTask) {
            setSelecting(true);
            selectAll(true);
          }
        } else if (selecting && !bulkTask && event.key === "Escape") {
          event.stopPropagation();
          stopSelecting();
        } else if (
          selecting &&
          !bulkTask &&
          (event.key === "Delete" || event.key === "Backspace")
        ) {
          event.preventDefault();
          event.stopPropagation();
          if (selected.size) setDeleteKeys([...selected]);
        }
      }}
    >
      <header className="pane-heading">
        <h1>
          {title}
          {!deleting && <span>{deletionRows.visibleCount}</span>}
        </h1>
        <button
          className="text-button list-select-toggle"
          data-testid="list.selectMode"
          aria-pressed={selecting}
          disabled={!!bulkTask || (!selecting && state.view.rows.length === 0)}
          onClick={() => {
            if (selecting) stopSelecting();
            else {
              setSelecting(true);
              void run("view.select", { keys: [], open: false });
            }
          }}
        >
          <ListChecks size={14} />
          {selecting ? t("完成") : t("选择")}
        </button>
        <div className="relative" ref={menuRoot}>
          <button
            className="text-button muted"
            aria-expanded={menu}
            data-testid="list.viewMenu"
            onClick={() => setMenu(!menu)}
          >
            {en ? "By date" : t("按时间排序")}
            <ChevronDown size={13} />
          </button>
          {menu && (
            <div className="popup-menu">
              <label>
                <input
                  type="checkbox"
                  checked={Boolean(state.prefs.groupBySubject)}
                  onChange={(e) => {
                    void run("app.prefs", { groupBySubject: e.target.checked });
                    setMenu(false);
                  }}
                />
                {en ? "Group by subject" : t("按主题归类")}
              </label>
              <label>
                <input
                  type="checkbox"
                  checked={compact}
                  onChange={(e) => {
                    void run("app.prefs", {
                      mailListDensity: e.target.checked
                        ? "compact"
                        : "comfortable",
                    });
                    setMenu(false);
                  }}
                />
                {t("紧凑列表")}
              </label>
              <button
                onClick={() => {
                  void run("folder.markAllRead", {});
                  setMenu(false);
                }}
              >
                {en ? "Mark all as read" : t("全部标为已读")}
              </button>
              <button
                data-testid="list.junk"
                onClick={() => {
                  onJunk();
                  setMenu(false);
                }}
              >
                {t("AI 清理垃圾邮件")}
              </button>
            </div>
          )}
        </div>
      </header>
      {selecting && (
        <div
          className="list-bulk-actions"
          role="toolbar"
          aria-label={t("批量操作")}
        >
          <label>
            <input
              type="checkbox"
              aria-label={t("全选当前列表")}
              data-testid="list.selectAll"
              checked={allSelected}
              ref={(node) => {
                if (node)
                  node.indeterminate = selected.size > 0 && !allSelected;
              }}
              disabled={!!bulkTask}
              onChange={(event) => selectAll(event.target.checked)}
            />
            {t("全选")}
          </label>
          <span className="bulk-count">
            {en ? `${selected.size} selected` : `已选 ${selected.size} 封`}
          </span>
          <button
            aria-label={t("标为已读")}
            data-testid="list.markRead"
            disabled={!!bulkTask || selected.size === 0}
            onClick={() => void bulkAction("message.markRead", [...selected])}
          >
            <CheckCheck size={14} />
            {t("已读")}
          </button>
          <button
            data-testid="list.delete"
            disabled={!!bulkTask || selected.size === 0}
            onClick={() => setDeleteKeys([...selected])}
          >
            <Trash2 size={14} />
            {t("删除")}
          </button>
        </div>
      )}
      <MailListRows
        items={deletionRows.items}
        selected={selected}
        compact={compact}
        english={en}
        selecting={selecting}
        disabled={!!bulkTask}
        scope={JSON.stringify([scope, state.view.query])}
        focused={state.view.focused}
        group={state.view.group}
        onToggle={(row, checked) => {
          void run("view.select", {
            keys: row.keys,
            extend: true,
            selected: checked,
            open: false,
          });
        }}
        onSelect={(row, extend) => {
          if (selecting || extend) {
            if (bulkTask) return;
            setSelecting(true);
            void run("view.select", {
              keys: row.keys,
              extend: true,
              selected: !row.keys.every((key) => selected.has(key)),
              open: false,
            });
            return;
          }
          void run(
            row.group && !extend ? "view.selectGroup" : "view.select",
            row.group && !extend
              ? { group: row.group }
              : { key: row.keys[0], extend },
          );
        }}
      >
        {deletionRows.items.length === 0 && (
          <div className="empty">
            <Inbox />
            <p>{en ? "No messages here" : t("这里还没有邮件")}</p>
            <button onClick={onCompose}>
              {en ? "Write a message" : t("写一封邮件")}
            </button>
          </div>
        )}
      </MailListRows>
      <footer className="list-footer">
        {deletionRows.active ? (
          en ? (
            `${deletionRows.visibleCount} messages`
          ) : (
            `共 ${deletionRows.visibleCount} 封邮件`
          )
        ) : bulkTask ? (
          bulkTask.action === "message.delete" ? (
            t("正在删除…")
          ) : (
            t("正在标为已读…")
          )
        ) : deleting ? (
          t("正在删除…")
        ) : state.status.busy ? (
          <>
            <LoaderCircle className="spin" size={12} />
            {en ? "Working…" : t("正在处理…")}
          </>
        ) : en ? (
          `${state.view.messages.length} messages`
        ) : (
          `共 ${state.view.messages.length} 封邮件`
        )}
      </footer>
      {deleteKeys &&
        createPortal(
          <Dialog title={t("删除邮件")} onClose={() => setDeleteKeys(null)}>
            <div className="settings-form">
              <p>
                {en
                  ? `Delete ${deleteKeys.length} selected messages?`
                  : `确定删除选中的 ${deleteKeys.length} 封邮件？`}
              </p>
              <p>
                {state.view.messages.some(
                  (m) =>
                    deleteTargets.has(keyString(m.key)) &&
                    state.folders[m.key.account]?.some(
                      (f) => f.path === m.key.folder && f.role === "trash",
                    ),
                )
                  ? t("已删除文件夹中的邮件将被永久删除，其余邮件移到已删除。")
                  : t("邮件将移到各自账户的已删除文件夹。")}
              </p>
              <footer>
                <button autoFocus onClick={() => setDeleteKeys(null)}>
                  {t("取消")}
                </button>
                <button
                  className="primary"
                  data-testid="list.confirmDelete"
                  onClick={() => void bulkAction("message.delete", deleteKeys)}
                >
                  {t("删除")}
                </button>
              </footer>
            </div>
          </Dialog>,
          document.body,
        )}
    </section>
  );
}
