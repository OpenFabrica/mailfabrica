import { useState } from "react";
import { createPortal } from "react-dom";
import { CheckCheck, ShieldAlert, Trash2 } from "lucide-react";
import { Dialog, folderName } from "./components";
import { useDismiss } from "./use-dismiss";
import { t } from "./i18n";
import type { Folder } from "./types";
import type { SessionUI } from "./use-session";

// FolderMenuTarget 保存操作对象，右键未选中的文件夹也不会改变当前邮件视图。
export interface FolderMenuTarget {
  account: string;
  folder: Folder;
  x: number;
  y: number;
}

// FolderMenu 通过统一命令处理整个文件夹，浮层放到 body 避免被侧栏滚动区域裁切。
export function FolderMenu({
  target,
  ui,
  onClose,
}: {
  target: FolderMenuTarget;
  ui: SessionUI;
  onClose: () => void;
}) {
  const [confirmDelete, setConfirmDelete] = useState(false);
  const root = useDismiss(!confirmDelete, onClose);
  const params = { account: target.account, folder: target.folder.path };
  const en = ui.state?.prefs.language === "en";
  const name = folderName(target.folder.path, target.folder.role, en);
  const permanent = target.folder.role === "trash";
  const removeLabel = permanent ? t("永久删除") : t("删除");
  const act = (command: string) => {
    void ui.run(command, params);
    onClose();
  };
  return createPortal(
    confirmDelete ? (
      <Dialog title={removeLabel} onClose={onClose}>
        <div className="settings-form">
          <p>
            {en
              ? `Delete all messages in “${name}”? ${permanent ? "This permanently deletes these messages." : "Messages will be moved to Trash."}`
              : `确定删除“${name}”中的全部邮件？${permanent ? "这些邮件将被永久删除。" : "邮件将移到“已删除”。"}`}
          </p>
          <footer>
            <button onClick={onClose} autoFocus>
              {t("取消")}
            </button>
            <button
              className="primary"
              data-testid="folder.confirmDelete"
              onClick={() => act("folder.deleteMessages")}
            >
              {removeLabel}
            </button>
          </footer>
        </div>
      </Dialog>
    ) : (
      <div
        ref={root}
        className="popup-menu folder-menu"
        aria-label={`${name} · ${t("文件夹操作")}`}
        style={{
          left: Math.max(8, Math.min(target.x, window.innerWidth - 218)),
          top: Math.max(8, Math.min(target.y, window.innerHeight - 136)),
        }}
      >
        <button
          data-testid="folder.markAllRead"
          onClick={() => act("folder.markAllRead")}
        >
          <CheckCheck size={15} />
          {t("全部标记为已读")}
        </button>
        <button
          data-testid="folder.deleteMessages"
          onClick={() => setConfirmDelete(true)}
        >
          <Trash2 size={15} />
          {removeLabel}
        </button>
        <button
          data-testid="folder.junk"
          disabled={target.folder.role === "junk"}
          onClick={() => act("folder.junk")}
        >
          <ShieldAlert size={15} />
          {t("移动到垃圾邮件")}
        </button>
      </div>
    ),
    document.body,
  );
}
