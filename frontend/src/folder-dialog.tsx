import { useState } from "react";
import { Dialog } from "./components";
import type { SessionUI } from "./use-session";

// FolderDialog 通过统一命令在所选账户创建文件夹，失败交由会话显示。
export function FolderDialog({
  ui,
  onClose,
}: {
  ui: SessionUI;
  onClose: () => void;
}) {
  const state = ui.state!;
  const en = state.prefs.language === "en";
  const [account, setAccount] = useState(
    state.view.scope.account || state.accounts[0]?.id || "",
  );
  const [name, setName] = useState("");
  const [saving, setSaving] = useState(false);
  return (
    <Dialog title={en ? "New folder" : "新建文件夹"} onClose={onClose}>
      <form
        className="settings-form"
        onSubmit={async (event) => {
          event.preventDefault();
          setSaving(true);
          const result = await ui.run("folder.create", {
            account,
            name: name.trim(),
          });
          setSaving(false);
          if (result !== undefined) onClose();
        }}
      >
        <label>
          {en ? "Account" : "邮箱账户"}
          <select value={account} onChange={(e) => setAccount(e.target.value)}>
            {state.accounts.map((a) => (
              <option key={a.id} value={a.id}>
                {a.name || a.email}
              </option>
            ))}
          </select>
        </label>
        <label>
          {en ? "Folder name" : "文件夹名称"}
          <input
            autoFocus
            required
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </label>
        <footer>
          <button type="button" onClick={onClose}>
            {en ? "Cancel" : "取消"}
          </button>
          <button
            className="primary"
            type="submit"
            disabled={!account || !name.trim() || saving}
          >
            {en ? "Create" : "创建"}
          </button>
        </footer>
      </form>
    </Dialog>
  );
}
